//! Chat text you can select and copy: styled runs (colors, code font) with
//! clickable links, taking part in the window-wide text selection that
//! gpui-base's `Root` draws. Drag across messages to select, Ctrl+C copies.
//!
//! gpui-base's own `SelectableText` can't carry font overrides or links, so
//! this is the same registration with those added.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::base::{TextSelection, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun};
use gpui_kit::*;

type LinkHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

pub struct RichText {
    id: ElementId,
    text: SharedString,
    styled: StyledText,
    order: u64,
    selection: Hsla,
    links: Vec<Range<usize>>,
    on_link: Option<LinkHandler>,
}

impl RichText {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>, selection: Hsla) -> Self {
        let text = text.into();
        Self {
            id: id.into(),
            styled: StyledText::new(text.clone()),
            text,
            order: 0,
            selection,
            links: Vec::new(),
            on_link: None,
        }
    }

    /// Reading order among all the selectable text in the window, so a
    /// selection across messages copies them top to bottom.
    pub fn order(mut self, order: u64) -> Self {
        self.order = order;
        self
    }

    pub fn styled(mut self, f: impl FnOnce(StyledText) -> StyledText) -> Self {
        self.styled = f(self.styled);
        self
    }

    /// Ranges that open something when clicked (not dragged over).
    pub fn links(mut self, ranges: Vec<Range<usize>>, on_click: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.links = ranges;
        self.on_link = Some(Rc::new(on_click));
        self
    }

    fn link_at(&self, position: Point<Pixels>) -> Option<usize> {
        let index = self.styled.layout().index_for_position(position).ok()?;
        self.links.iter().position(|r| r.contains(&index))
    }
}

/// Kept between frames: the selection handle and where a press began.
#[derive(Clone)]
pub struct Retained {
    handle: TextSelectionHandle,
    pressed: Rc<Cell<Option<Point<Pixels>>>>,
    _refresh: Rc<Subscription>,
}

impl Element for RichText {
    type RequestLayoutState = Retained;
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
        let text = self.text.clone();
        let retained = window.with_element_state(global_id.expect("RichText has an id"), |kept: Option<Retained>, window| {
            let kept = kept.unwrap_or_else(|| {
                let handle = TextSelectionHandle::new(text.clone(), cx);
                let refresh = handle.refresh_window_on_change(window, cx);
                Retained { handle, pressed: Rc::default(), _refresh: Rc::new(refresh) }
            });
            (kept.clone(), kept)
        });
        retained.handle.set_fallback_copy_text(self.text.to_string(), cx);
        let (layout_id, ()) = self.styled.request_layout(global_id, inspector_id, window, cx);
        (layout_id, retained)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        retained: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled.prepaint(global_id, inspector_id, bounds, &mut (), window, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds)
            .with_document_order(self.order)
            .with_text_bounds(vec![bounds])
            .with_rendered_element(&retained.handle, window, cx);
        retained.handle.register(registration, window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        retained: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled.layout().clone();
        let projection = retained
            .handle
            .update_runs(&[TextSelectionRun::new(self.text.clone(), layout.clone(), bounds).with_document_order(self.order)], cx);
        for range in projection.ranges().iter().flatten().cloned() {
            paint_selection(&layout, range, self.selection, window);
        }

        let over_link = hitbox.is_hovered(window) && self.link_at(window.mouse_position()).is_some();
        window.set_cursor_style(if over_link { CursorStyle::PointingHand } else { CursorStyle::IBeam }, hitbox);
        if let Some(on_link) = self.on_link.clone() {
            // A link opens on a click, not at the end of a drag that selects.
            let pressed = retained.pressed.clone();
            let hb = hitbox.clone();
            window.on_mouse_event(move |e: &MouseDownEvent, phase, window, _| {
                if phase == DispatchPhase::Bubble && e.button == MouseButton::Left && hb.is_hovered(window) {
                    pressed.set(Some(e.position));
                }
            });
            let pressed = retained.pressed.clone();
            let hb = hitbox.clone();
            let links = self.links.clone();
            let layout = layout.clone();
            window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || e.button != MouseButton::Left {
                    return;
                }
                let Some(start) = pressed.take() else { return };
                let moved = (e.position.x - start.x).abs() + (e.position.y - start.y).abs();
                if !hb.is_hovered(window) || moved > px(4.) || TextSelection::has_selection(window, cx) {
                    return;
                }
                let Ok(index) = layout.index_for_position(e.position) else { return };
                if let Some(ix) = links.iter().position(|r| r.contains(&index)) {
                    on_link(ix, window, cx);
                }
            });
        }
        self.styled.paint(global_id, inspector_id, bounds, &mut (), &mut (), window, cx);
    }
}

fn paint_selection(layout: &TextLayout, range: Range<usize>, color: Hsla, window: &mut Window) {
    let (Some(start), Some(end)) = (layout.position_for_index(range.start), layout.position_for_index(range.end)) else {
        return;
    };
    let line = layout.line_height();
    let b = layout.bounds();
    let mut quads = Vec::new();
    if start.y == end.y {
        quads.push(Bounds::from_corners(start, point(end.x, end.y + line)));
    } else {
        quads.push(Bounds::from_corners(start, point(b.right(), start.y + line)));
        if end.y > start.y + line {
            quads.push(Bounds::from_corners(point(b.left(), start.y + line), point(b.right(), end.y)));
        }
        quads.push(Bounds::from_corners(point(b.left(), end.y), point(end.x, end.y + line)));
    }
    for q in quads {
        window.paint_quad(fill(q, color));
    }
}
