import pathlib
import shutil
import subprocess
import tempfile
import unittest

JUSTFILE = pathlib.Path(__file__).resolve().parents[1] / "justfile"


def install_file(source, target):
    subprocess.run(
        ["just", "--justfile", str(JUSTFILE), "_install-file", str(source), str(target), "0755"],
        check=True, capture_output=True, text=True,
    )


class InstallTests(unittest.TestCase):
    def test_upgrade_keeps_running_process_and_replaces_next_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            target = pathlib.Path(directory) / "terminal"
            shutil.copy2(shutil.which("sleep"), target)
            child = subprocess.Popen([str(target), "60"])
            try:
                install_file(pathlib.Path(shutil.which("false")), target)
                self.assertIsNone(child.poll())
                self.assertEqual(subprocess.run([str(target)], check=False).returncode, 1)
            finally:
                child.terminate()
                child.wait(timeout=5)

    def test_failed_copy_preserves_installed_file(self):
        with tempfile.TemporaryDirectory() as directory:
            target = pathlib.Path(directory) / "terminal"
            target.write_bytes(b"previous installation")
            with self.assertRaises(subprocess.CalledProcessError):
                install_file(target.with_name("missing"), target)
            self.assertEqual(target.read_bytes(), b"previous installation")

    def test_desktop_launch_preserves_quoted_executable_path(self):
        with tempfile.TemporaryDirectory() as directory:
            target = pathlib.Path(directory) / "space ' quote \" slash \\ dollar $ tick `" / "terminal"
            target.parent.mkdir()
            target.write_text("#!/bin/sh\nprintf quoted-launch-ok\n")
            target.chmod(0o755)
            desktop = pathlib.Path(directory) / "terminal.desktop"
            subprocess.run(
                ["just", "--justfile", str(JUSTFILE), "_install-desktop",
                 str(JUSTFILE.parent / "resources/app.desktop"), str(desktop), str(target)],
                check=True, capture_output=True, text=True,
            )
            result = subprocess.run(
                ["gio", "launch", str(desktop)], check=True, capture_output=True, text=True,
            )
            self.assertEqual(result.stdout, "quoted-launch-ok")

    def test_invalid_desktop_path_preserves_previous_entry(self):
        with tempfile.TemporaryDirectory() as directory:
            desktop = pathlib.Path(directory) / "terminal.desktop"
            shutil.copyfile(JUSTFILE.parent / "resources/app.desktop", desktop)
            previous = desktop.read_bytes()
            result = subprocess.run(
                ["just", "--justfile", str(JUSTFILE), "_install-desktop",
                 str(JUSTFILE.parent / "resources/app.desktop"), str(desktop), "/invalid\npath"],
                check=False, capture_output=True, text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(desktop.read_bytes(), previous)


if __name__ == "__main__":
    unittest.main()
