// SPDX-License-Identifier: GPL-3.0-only
//! Private, single-owner control endpoint. The lock inode is never unlinked: doing so
//! would let two launchers lock different inodes and both become the owner.
use cosmic::iced::{self, Subscription, stream};
use std::os::unix::process::CommandExt;
use std::{
    env,
    fs::{self, DirBuilder, File, OpenOptions},
    hash::{Hash, Hasher},
    io::{self, BufRead, BufReader, Read, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

pub fn watch_display_disconnect() -> io::Result<()> {
    let socket = if let Some(fd) = env::var_os("WAYLAND_SOCKET") {
        let fd: i32 = fd
            .to_string_lossy()
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid WAYLAND_SOCKET"))?;
        // SAFETY: fcntl validates the descriptor; duplicating does not consume the
        // descriptor that the GUI will take from WAYLAND_SOCKET.
        let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
        if duplicate < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful F_DUPFD_CLOEXEC returned a fresh, owned descriptor.
        UnixStream::from(unsafe { OwnedFd::from_raw_fd(duplicate) })
    } else {
        let display =
            PathBuf::from(env::var_os("WAYLAND_DISPLAY").unwrap_or_else(|| "wayland-0".into()));
        let path = if display.is_absolute() {
            display
        } else {
            PathBuf::from(env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set")
            })?)
            .join(display)
        };
        UnixStream::connect(path)?
    };
    thread::Builder::new()
        .name("wayland-lifetime".into())
        .spawn(move || {
            let result = wait_for_display_disconnect(socket);
            eprintln!("Wayland connection closed; stopping terminal backend: {result:?}");
            // libcosmic's SCTK loop currently logs fatal dispatch errors and continues.
            // Its GUI executor cannot reliably process an exit task after disconnect.
            // Process exit also releases the control lock and closes all PTY masters.
            std::process::exit(1);
        })?;
    Ok(())
}

fn wait_for_display_disconnect(socket: UnixStream) -> io::Result<()> {
    let mut descriptor = libc::pollfd {
        fd: socket.as_raw_fd(),
        events: 0,
        revents: 0,
    };
    loop {
        // SAFETY: descriptor points to one initialized pollfd; socket owns its fd
        // throughout this call. Request no readable events: never consume GUI data.
        let result = unsafe { libc::poll(&mut descriptor, 1, -1) };
        if result > 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Toggle,
    ToggleVisibility,
    Show,
    Hide,
    Status,
    Reload,
    Quit,
}
impl Command {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "toggle" => Self::Toggle,
            "toggle-visibility" => Self::ToggleVisibility,
            "show" => Self::Show,
            "hide" => Self::Hide,
            "status" => Self::Status,
            "reload" => Self::Reload,
            "quit" => Self::Quit,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::ToggleVisibility => "toggle-visibility",
            Self::Show => "show",
            Self::Hide => "hide",
            Self::Status => "status",
            Self::Reload => "reload",
            Self::Quit => "quit",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Request {
    pub command: Command,
    pub reply: mpsc::Sender<&'static str>,
}

#[derive(Debug)]
pub struct Server {
    listener: UnixListener,
    _lock: File,
    socket: PathBuf,
}
impl Hash for Server {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.socket.hash(state);
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
    }
}

fn private_directory(path: &Path) -> io::Result<()> {
    match DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => (),
        Err(err) => return Err(err),
    }
    let metadata = fs::symlink_metadata(path)?;
    // SAFETY: geteuid has no preconditions and does not mutate process state.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o777 != 0o700 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} must be an owned directory with mode 0700",
                path.display()
            ),
        ));
    }
    Ok(())
}

pub fn directory() -> io::Result<PathBuf> {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
    let runtime = PathBuf::from(runtime);
    if !runtime.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "XDG_RUNTIME_DIR must be absolute",
        ));
    }
    let metadata = fs::symlink_metadata(&runtime)?;
    // SAFETY: geteuid has no preconditions.
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "XDG_RUNTIME_DIR must be private and owned by this user",
        ));
    }
    let directory = runtime.join("cosmos-applet-dropdown-terminal-backend");
    private_directory(&directory)?;
    Ok(directory)
}

impl Server {
    pub fn bind(directory: &Path) -> io::Result<Option<Self>> {
        private_directory(directory)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory.join("owner.lock"))?;
        match lock.try_lock() {
            Ok(()) => (),
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(std::fs::TryLockError::Error(err)) => return Err(err),
        }
        let socket = directory.join("control.sock");
        match fs::symlink_metadata(&socket) {
            Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(&socket)?,
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "control.sock is not a socket",
                ));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => (),
            Err(err) => return Err(err),
        }
        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        Ok(Some(Self {
            listener,
            _lock: lock,
            socket,
        }))
    }

    pub fn subscription(self: &Arc<Self>) -> Subscription<crate::Message> {
        Subscription::run_with(self.clone(), |server| {
            let listener = server.listener.try_clone();
            stream::channel(
                32,
                move |mut output: iced::futures::channel::mpsc::Sender<crate::Message>| async move {
                    thread::spawn(move || {
                        let listener = match listener {
                            Ok(value) => value,
                            Err(err) => {
                                log::error!("control listener: {err}");
                                return;
                            }
                        };
                        for connection in listener.incoming() {
                            let mut socket = match connection {
                                Ok(value) => value,
                                Err(err) => {
                                    log::error!("control accept: {err}");
                                    continue;
                                }
                            };
                            let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
                            let _ = socket.set_write_timeout(Some(Duration::from_secs(2)));
                            let mut line = String::new();
                            let read = BufReader::new((&socket).take(65)).read_line(&mut line);
                            if read.is_err() || !line.ends_with('\n') || line.len() > 64 {
                                let _ = socket
                                    .write_all(b"error expected one command followed by newline\n");
                                continue;
                            }
                            let Some(command) = Command::parse(line.trim_end_matches(['\r', '\n']))
                            else {
                                let _ = socket.write_all(b"error unknown command\n");
                                continue;
                            };
                            let (reply, response) = mpsc::channel();
                            if output
                                .try_send(crate::Message::Control(Request { command, reply }))
                                .is_err()
                            {
                                let _ =
                                    socket.write_all(b"error backend control queue unavailable\n");
                                break;
                            }
                            let answer = response
                                .recv_timeout(Duration::from_secs(5))
                                .unwrap_or("error backend did not respond");
                            let _ = writeln!(socket, "{answer}");
                            if command == Command::Quit {
                                let _ = output.try_send(crate::Message::QuitAcknowledged);
                                break;
                            }
                        }
                    });
                    std::future::pending::<()>().await;
                },
            )
        })
    }
}

fn wait_for_shutdown(
    directory: &Path,
    original_socket: Option<&File>,
    timeout: Duration,
) -> io::Result<()> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join("owner.lock"))?;
    let original_identity = original_socket
        .map(File::metadata)
        .transpose()?
        .map(|metadata| (metadata.dev(), metadata.ino()));
    let deadline = Instant::now() + timeout;
    loop {
        match lock.try_lock() {
            // Holding the original lock proves the old owner has released it.
            // Drop our handle before returning so the next launcher can own it.
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) => (),
            Err(std::fs::TryLockError::Error(err)) => return Err(err),
        }
        match fs::symlink_metadata(directory.join("control.sock")) {
            Ok(metadata) if Some((metadata.dev(), metadata.ino())) != original_identity => {
                // A replacement can bind only after the old owner's lock releases.
                // Its lifetime must not extend this quit command's wait.
                return Ok(());
            }
            Ok(_) => (),
            Err(err) if err.kind() == io::ErrorKind::NotFound => (),
            Err(err) => return Err(err),
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "backend acknowledged quit but did not release ownership",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn request(directory: &Path, command: Command) -> io::Result<String> {
    let mut socket = UnixStream::connect(directory.join("control.sock"))?;
    let original_socket = if command == Command::Quit {
        // O_PATH retains the socket inode through unlink, preventing inode reuse
        // from making a replacement look like the old endpoint.
        match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory.join("control.sock"))
        {
            Ok(file) => Some(file),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err),
        }
    } else {
        None
    };
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    writeln!(socket, "{}", command.name())?;
    let mut response = String::new();
    BufReader::new(socket.take(256)).read_line(&mut response)?;
    if !response.ends_with('\n') {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "backend closed without a response",
        ));
    }
    if response.starts_with("error ") {
        return Err(io::Error::other(response.trim().to_owned()));
    }
    if command == Command::Quit {
        wait_for_shutdown(directory, original_socket.as_ref(), Duration::from_secs(5))?;
    }
    Ok(response)
}

pub fn client(command: Command) -> io::Result<()> {
    client_inner(command, false)
}

pub fn start() -> io::Result<()> {
    client_inner(Command::Status, true)
}

fn client_inner(command: Command, ensure_running: bool) -> io::Result<()> {
    let directory = directory()?;
    match request(&directory, command) {
        Ok(response) => {
            print!("{response}");
            return Ok(());
        }
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) => {}
        Err(err) => return Err(err),
    }
    if !ensure_running
        && matches!(
            command,
            Command::Status | Command::Hide | Command::Reload | Command::Quit
        )
    {
        println!("stopped");
        return Ok(());
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join("backend.log"))?;
    let mut process = ProcessCommand::new(env::current_exe()?);
    process
        .arg("--daemon")
        .env_remove("WAYLAND_SOCKET")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    // SAFETY: setsid is async-signal-safe; this closure executes only between fork and exec.
    unsafe {
        process.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = process.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match request(&directory, command) {
            Ok(response) => {
                print!("{response}");
                return Ok(());
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) && Instant::now() < deadline => {}
            Err(err) => return Err(err),
        }
        if let Some(status) = child.try_wait()?
            && !status.success()
        {
            return Err(io::Error::other(format!("backend exited: {status}")));
        }
        // A successful child may have lost ownership to another launcher still starting.
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    fn directory() -> PathBuf {
        let path = env::temp_dir().join(format!(
            "cosmos-dropdown-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        private_directory(&path).unwrap();
        path
    }
    #[test]
    fn only_one_owner_can_bind_and_owner_drop_allows_restart() {
        let path = directory();
        let owner = Server::bind(&path).unwrap().unwrap();
        assert!(Server::bind(&path).unwrap().is_none());
        assert_eq!(
            fs::metadata(path.join("control.sock")).unwrap().mode() & 0o777,
            0o600
        );
        drop(owner);
        assert!(Server::bind(&path).unwrap().is_some());
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn stale_socket_is_recovered_only_after_lock_is_owned() {
        let path = directory();
        drop(UnixListener::bind(path.join("control.sock")).unwrap());
        assert!(Server::bind(&path).unwrap().is_some());
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn refuses_symlink_and_public_directory() {
        let path = directory();
        let target = path.with_extension("link");
        std::os::unix::fs::symlink(&path, &target).unwrap();
        assert!(private_directory(&target).is_err());
        fs::remove_file(target).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_directory(&path).is_err());
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn accepts_only_documented_commands() {
        for name in [
            "toggle",
            "toggle-visibility",
            "show",
            "hide",
            "status",
            "reload",
            "quit",
        ] {
            assert_eq!(Command::parse(name).unwrap().name(), name);
        }
        assert_eq!(Command::parse("toggle show"), None);
    }
    #[test]
    fn quit_waits_for_owner_release_after_bye() {
        let path = directory();
        let owner = Server::bind(&path).unwrap().unwrap();
        let (completed, completion) = mpsc::channel();
        let client_path = path.clone();
        let client = thread::spawn(move || {
            completed
                .send(request(&client_path, Command::Quit))
                .unwrap();
        });
        let (mut socket, _) = owner.listener.accept().unwrap();
        let mut command = String::new();
        BufReader::new(socket.try_clone().unwrap())
            .read_line(&mut command)
            .unwrap();
        assert_eq!(command, "quit\n");
        socket.write_all(b"bye\n").unwrap();
        assert!(matches!(
            completion.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(owner);
        assert_eq!(
            completion
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap(),
            "bye\n"
        );
        client.join().unwrap();
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn quit_does_not_wait_for_a_replacement_owner() {
        let path = directory();
        let owner = Server::bind(&path).unwrap().unwrap();
        let old_socket = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path.join("control.sock"))
            .unwrap();
        drop(owner);
        let replacement = Server::bind(&path).unwrap().unwrap();
        wait_for_shutdown(&path, Some(&old_socket), Duration::ZERO).unwrap();
        assert!(Server::bind(&path).unwrap().is_none());
        drop(replacement);
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn quit_wait_is_bounded_when_owner_never_releases() {
        let path = directory();
        let owner = Server::bind(&path).unwrap().unwrap();
        let socket = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path.join("control.sock"))
            .unwrap();
        let error = wait_for_shutdown(&path, Some(&socket), Duration::ZERO).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        drop(owner);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn display_monitor_ignores_readable_data_and_finishes_on_disconnect() {
        let (mut client, mut compositor) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let monitor = client.try_clone().unwrap();
        let (closed, completion) = mpsc::channel();
        let worker = thread::spawn(move || {
            closed.send(wait_for_display_disconnect(monitor)).unwrap();
        });
        compositor.write_all(b"wire").unwrap();
        assert!(matches!(
            completion.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        let mut bytes = [0; 4];
        client.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"wire");
        drop(compositor);
        completion
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        worker.join().unwrap();
    }
}
