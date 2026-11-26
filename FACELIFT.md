# Hermes Bot — Grok-aligned facelift

Visual refresh for the native GPUI client. Scope: look and feel only. Target: **grok.com** patterns (warm chrome, composer island, history-style sidebar) mapped onto Hermes bots + servers.

**Status:** implementing on branch `cursor/facelift-proposal-a423`

---

## Design direction (Grok official as reference)

| Grok token | Hermes built-in dark |
|------------|---------------------|
| Chrome `#1E1D1D` | `background` = `surface` (unified shell) |
| Pane `#282726` | `raised` (composer island) |
| Base `#E6E5E3` | **Primary** (send disc)—not launcher yellow |
| Accent `#1084FE` | Focus ring, links, list markers in markdown |
| Yellow | **Omarchy** theme choice only |

Layout: sidebar **256px**, column **764px** max, composer **22px** island radius, bot rows **40px** wash hover (no accent bar). Empty chat: time-of-day greeting + starter rows that prefill the composer.

Implementation map: `backend/theme.rs`, `ui/theme.rs`, `ui/root.rs`, `ui/chat.rs`, `ui/composer.rs`, `ui/widgets.rs`.

See [PLAN.md](PLAN.md) for product scope.
