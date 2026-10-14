//! Mouse-selectable text (drag + Ctrl+C), with optional highlighting.
//!
//! Adapted from `gpui_base::SelectableText` (MIT/Apache-2.0), which doesn't support `HighlightStyle`:
//! here JSON and search matches stay colored. Selection and copy are handled by
//! `gpui_base::TextSelection`; the window must paint a `TextSelectionLayer`.

use std::ops::Range;

use gpui_kit::base::{
    TextSelection, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun,
};
use gpui_kit::*;

pub struct SelText {
    id: ElementId,
    handle: Option<TextSelectionHandle>,
    text: SharedString,
    styled_text: StyledText,
    document_order: u64,
}

impl SelText {
    /// Text with its own selection.
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        Self {
            id: id.into(),
            handle: None,
            styled_text: StyledText::new(text.clone()),
            text,
            document_order: 0,
        }
    }

    /// Text that forms a single document with the others of `handle`: it can be selected
    /// by dragging from one to another (several console lines, several headers…).
    pub fn shared(
        id: impl Into<ElementId>,
        handle: &TextSelectionHandle,
        order: u64,
        text: impl Into<SharedString>,
    ) -> Self {
        let mut t = Self::new(id, text);
        t.handle = Some(handle.clone());
        t.document_order = order;
        t
    }

    pub fn highlights(mut self, highlights: Vec<(Range<usize>, HighlightStyle)>) -> Self {
        self.styled_text = StyledText::new(self.text.clone()).with_highlights(highlights);
        self
    }
}

fn selection_quads(
    start: Point<Pixels>,
    end: Point<Pixels>,
    bounds: Bounds<Pixels>,
    line: Pixels,
) -> Vec<Bounds<Pixels>> {
    if start.y == end.y {
        return vec![Bounds::from_corners(start, point(end.x, end.y + line))];
    }
    let mut quads = vec![Bounds::from_corners(
        start,
        point(bounds.right(), start.y + line),
    )];
    if end.y > start.y + line {
        quads.push(Bounds::from_corners(
            point(bounds.left(), start.y + line),
            point(bounds.right(), end.y),
        ));
    }
    quads.push(Bounds::from_corners(
        point(bounds.left(), end.y),
        point(end.x, end.y + line),
    ));
    quads
}

impl IntoElement for SelText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelText {
    type RequestLayoutState = TextSelectionHandle;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let handle = self.handle.clone().unwrap_or_else(|| {
            window.with_element_state(
                global_id.expect("SelText requires an id"),
                |retained: Option<TextSelectionHandle>, _| {
                    let handle =
                        retained.unwrap_or_else(|| TextSelectionHandle::new(self.text.clone(), cx));
                    (handle.clone(), handle)
                },
            )
        });
        let (layout_id, ()) = self
            .styled_text
            .request_layout(global_id, inspector_id, window, cx);
        (layout_id, handle)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled_text
            .prepaint(global_id, inspector_id, bounds, &mut (), window, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds)
            .with_document_order(self.document_order)
            .with_text_bounds(vec![bounds])
            .with_rendered_element(handle, window, cx);
        handle.register(registration, window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled_text.layout().clone();
        let before = TextSelection::selected_text(window, cx);
        let projection = handle.update_runs(
            &[
                TextSelectionRun::new(self.text.clone(), layout.clone(), bounds)
                    .with_document_order(self.document_order),
            ],
            cx,
        );
        if before != TextSelection::selected_text(window, cx) {
            window.refresh();
        }
        let color: Hsla = crate::theme::rgb(crate::theme::ACCENT_WASH).into();
        for range in projection.ranges().iter().flatten().cloned() {
            if let (Some(start), Some(end)) = (
                layout.position_for_index(range.start),
                layout.position_for_index(range.end),
            ) {
                for quad in selection_quads(start, end, layout.bounds(), layout.line_height()) {
                    window.paint_quad(fill(quad, color));
                }
            }
        }
        self.styled_text.paint(
            global_id,
            inspector_id,
            bounds,
            &mut (),
            &mut (),
            window,
            cx,
        );
    }
}

/// Selection document: distributes the reading order among its texts.
#[derive(Clone)]
pub struct SelDoc {
    prefix: &'static str,
    handle: TextSelectionHandle,
    next: std::rc::Rc<std::cell::Cell<u64>>,
}

impl SelDoc {
    /// `base`: first order (so two documents with the same `handle` don't collide).
    pub fn new(prefix: &'static str, handle: &TextSelectionHandle, base: u64) -> Self {
        Self {
            prefix,
            handle: handle.clone(),
            next: std::rc::Rc::new(std::cell::Cell::new(base)),
        }
    }

    pub fn text(&self, text: impl Into<SharedString>) -> SelText {
        let order = self.next.get();
        self.next.set(order + 1);
        SelText::shared(
            SharedString::from(format!("{}-{order}", self.prefix)),
            &self.handle,
            order,
            text,
        )
    }
}
