// SPDX-License-Identifier: GPL-3.0-only
use cosmic::{
    Element, Renderer, Theme,
    iced::{
        Event, Length, Rectangle, Size, Vector,
        core::{
            Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
            widget::{Operation, Tree, tree},
        },
        window,
    },
};

use crate::Message;

pub const HEIGHT: f32 = 24.0;

/// Captures the entire drag, including motion and release outside the grip.
/// Wayland's implicit pointer grab keeps delivery on this surface until release.
pub fn resize_grip<'a>(
    content: Element<'a, Message>,
    id: window::Id,
    resizing: bool,
) -> Element<'a, Message> {
    Element::new(ResizeGrip {
        content,
        id,
        resizing,
    })
}

struct ResizeGrip<'a> {
    content: Element<'a, Message>,
    id: window::Id,
    resizing: bool,
}

#[derive(Default)]
struct State {
    dragging: bool,
    size: Option<Size>,
}

impl Widget<Message, Theme, Renderer> for ResizeGrip<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.state.downcast_mut::<State>().dragging = self.resizing;
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        // Rendered bounds remain authoritative when layer resize events are stale.
        let size = viewport.size();
        if matches!(event, Event::Window(window::Event::RedrawRequested(_)))
            && state.size != Some(size)
        {
            state.size = Some(size);
            shell.publish(Message::SurfaceResized(self.id, size));
            shell.request_redraw();
        }
        let dragging = &mut state.dragging;
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.is_over(grip_bounds(layout.bounds())) =>
            {
                if let Some(position) = cursor.position() {
                    *dragging = true;
                    shell.publish(Message::DropdownResizeStart(self.id, position.y));
                    shell.request_redraw();
                    shell.capture_event();
                    return;
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if *dragging => {
                shell.publish(Message::DropdownResizeMotion(self.id, position.y));
                shell.request_redraw();
                shell.capture_event();
                return;
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if *dragging => {
                *dragging = false;
                shell.request_redraw();
                shell.capture_event();
                return;
            }
            Event::Mouse(_) if *dragging => {
                shell.capture_event();
                return;
            }
            _ => (),
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if tree.state.downcast_ref::<State>().dragging
            || cursor.is_over(grip_bounds(layout.bounds()))
        {
            mouse::Interaction::ResizingVertically
        } else {
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                layout,
                cursor,
                viewport,
                renderer,
            )
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }

    fn drag_destinations(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        rectangles: &mut cosmic::iced::core::clipboard::DndDestinationRectangles,
    ) {
        self.content
            .as_widget()
            .drag_destinations(&tree.children[0], layout, renderer, rectangles);
    }
}

fn grip_bounds(bounds: Rectangle) -> Rectangle {
    let height = HEIGHT.min(bounds.height);
    Rectangle {
        y: bounds.y + bounds.height - height,
        height,
        ..bounds
    }
}
