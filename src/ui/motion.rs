//! Motion: the transitions views share and the placeholders shown while
//! things load. Every animation goes through GPUI's `AnimationExt`, which
//! renders the end state (or the first frame of a loop) when the system asks
//! for reduced motion.

use std::time::{Duration, Instant};

use gpui_kit::*;

use super::theme::Colors;

/// Small things: menus, chips, a check mark.
pub const QUICK: Duration = Duration::from_millis(140);
/// Most entrances: messages, cards, dialogs.
pub const BASE: Duration = Duration::from_millis(220);
/// Bigger surfaces: the side panel, a whole conversation.
pub const SLOW: Duration = Duration::from_millis(320);
/// How long a placeholder takes to breathe in and out.
const PULSE: Duration = Duration::from_millis(1600);

/// Decelerates into place (cubic ease-out).
pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// A soft, slightly springy settle for things that move across the window.
pub fn settle() -> SpringConfig {
    SpringConfig::new(420.0, 38.0, 1.0)
}

/// Whether something that happened at `at` is recent enough to animate.
///
/// The chat list renders only the rows on screen, so a row's animation
/// state is dropped when it scrolls away; asking this first keeps an old row
/// from replaying its entrance when it comes back into view.
pub fn fresh(at: Option<Instant>, within: Duration) -> bool {
    at.is_some_and(|t| t.elapsed() < within)
}

/// Fades an element in while it rises `lift` pixels into place. The element
/// becomes relatively positioned; don't use it on absolute ones.
pub fn rise_in<E: Styled + IntoElement + 'static>(
    el: E,
    key: impl Into<ElementId>,
    lift: f32,
    duration: Duration,
) -> AnimationElement<E> {
    el.with_animation(key, Animation::new(duration).with_easing(ease_out), move |el, t| {
        el.opacity(t).relative().top(px(lift * (1.0 - t)))
    })
}

/// `rise_in` when `enabled`, otherwise the element as it is. The wrapper is
/// there either way, so the element's identity (and the state of everything
/// inside it) doesn't change when the entrance stops applying.
pub fn rise_in_when<E: Styled + IntoElement + 'static>(
    enabled: bool,
    el: E,
    key: impl Into<ElementId>,
    lift: f32,
    duration: Duration,
) -> AnimationElement<E> {
    el.with_animation(key, Animation::new(duration).with_easing(ease_out), move |el, t| {
        if enabled { el.opacity(t).relative().top(px(lift * (1.0 - t))) } else { el }
    })
}

/// Fades an element in.
pub fn fade_in<E: Styled + IntoElement + 'static>(el: E, key: impl Into<ElementId>, duration: Duration) -> AnimationElement<E> {
    el.with_animation(key, Animation::new(duration).with_easing(ease_out), |el, t| el.opacity(t))
}

/// A soft pulse for "this is still going" (a spinner's quieter cousin).
pub fn breathe<E: Styled + IntoElement + 'static>(el: E, key: impl Into<ElementId>, low: f32) -> AnimationElement<E> {
    el.with_animation(
        key,
        Animation::new(PULSE).repeat_synced().with_easing(pulsating_between(low, 1.0)),
        |el, t| el.opacity(t),
    )
}

/// A placeholder block standing in for content that's loading. Every
/// placeholder shares one clock, so a screenful of them pulses together.
pub fn skeleton(key: impl Into<ElementId>, width: impl Into<Length>, height: f32, c: &Colors) -> impl IntoElement {
    breathe(div().w(width.into()).h(px(height)).flex_none().rounded(c.radius).bg(c.foreground.alpha(0.09)), key, 0.35)
}

/// A round placeholder (an avatar, a status dot).
pub fn skeleton_circle(key: impl Into<ElementId>, size: f32, c: &Colors) -> impl IntoElement {
    breathe(div().size(px(size)).flex_none().rounded_full().bg(c.foreground.alpha(0.09)), key, 0.35)
}

/// Placeholder lines of text; the last is shorter, as paragraphs end.
pub fn skeleton_lines(key: &str, lines: usize, line_height: f32, c: &Colors) -> impl IntoElement {
    div().w_full().flex().flex_col().gap(px(line_height * 0.6)).children((0..lines).map(|i| {
        let width = if i + 1 == lines && lines > 1 { 0.55 } else { 1.0 - (i % 3) as f32 * 0.06 };
        skeleton(ElementId::Name(format!("{key}-line-{i}").into()), relative(width), line_height, c)
    }))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{BASE, ease_out, fresh};

    #[test]
    fn ease_out_starts_fast_and_lands_on_one() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.5);
        assert_eq!(ease_out(2.0), 1.0);
    }

    #[test]
    fn fresh_only_for_recent_moments() {
        assert!(!fresh(None, BASE));
        assert!(fresh(Some(Instant::now()), BASE));
        let old = Instant::now().checked_sub(Duration::from_secs(5)).unwrap();
        assert!(!fresh(Some(old), BASE));
    }
}
