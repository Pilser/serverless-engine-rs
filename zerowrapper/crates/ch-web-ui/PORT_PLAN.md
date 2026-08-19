# Astro → Topcoat Port Plan

## Design System
- Magenta accent hsl(340,100%,50%), green borders hsl(120,70%,40%)
- Light/dark theme via `data-theme` + localStorage + prefers-color-scheme
- Inter font, canvas dot-grid background with sine wave animation

## Current State (July 30, 2026)
**Phase 1 — Design System + Layout + Header + Footer: DONE** (compiles)
- `styles.css`: Astro exact palette (light + dark), canvas CSS, page-root border, plus all ChatApp message/markdown/table/thinking/tool-call/lightbox/embed CSS
- `layout.rs`: Canvas BG, theme init script (FOUC prevention), animated dots, importmap for marked+highlight.js, main.js + owner.js loaded, landing/settings JSON data scripts, owner_chat component rendered, body class/styles
- `header.rs`: Sticky, desktop nav, theme toggle (sun/moon SVGs), mobile hamburger + panel, outside-click close
- `footer.rs`: Flex column/row, dynamic links from YAML, hover styles, mobile border removal
- `config.rs`: Runtime YAML reader (serde_yaml), SiteSettings + LandingData structs (all Serialize)
- `context.rs`: AppContext holds settings + landing
- `main.rs`: YAML loading with fallback defaults, wired into AppContext
- Old unused components deleted (hero, faq, chat_app, chat_message, owner_login, owner_chat, typing_indicator, 12 UI primitives)

**Phase 2 — Chat Page SSR: DONE**
- `routes/chat.rs` has complete HTML with all IDs matching Astro (example-chips, chat-messages, typing-indicator, auth-section, user-badge, chat-form, send-btn, cancel-btn, FAQ accordion with toggle JS, hero text)
- Chat is served at `/chat` route
- No separate Topcoat sub-components needed — AuthSection, ChatForm, ExampleChips, UserBadge, TypingIndicator are all JS-constructed from mounting points

**Phase 3 — Chat JS Modules: DONE** (14 files copied from Astro, exact originals)
- `assets/js/chat/main.js` — entry point, imports all others
- `assets/js/chat/network.js` — WS state machine + reconnect
- `assets/js/chat/dom.js` — DOM element references
- `assets/js/chat/auth.js` — auth state + localStorage
- `assets/js/chat/connection.js` — WS message routing
- `assets/js/chat/messages.js` — DOM message builder + history
- `assets/js/chat/streaming.js` — rAF-batched markdown streaming
- `assets/js/chat/markdown.js` — HLJS highlight + table wrap + lazy load
- `assets/js/chat/tool-calls.js` — tool call pill UI
- `assets/js/chat/lightbox.js` — image lightbox
- `assets/js/chat/file-upload.js` — file upload + attachment chip
- `assets/js/chat/constants.js` — SVG icons + escapeHtml
- `assets/js/chat/owner.js` — owner login/chat JS
- `assets/js/chat/owner-chat.js` — owner-specific chat interactions
- `assets/js/owner.js` — top-level owner JS (loaded by layout)

**Phase 4 — Owner System: DONE**
- `src/components/owner_login.rs` — login overlay (form + contact info panels, toggle JS, inline styles)
- `src/components/owner_chat.rs` — full-screen owner chat (header, role badge, messages area, typing indicator 3-dot animation, input bar with upload/send/cancel, message CSS)
- `src/routes/owner.rs` — renders owner_login at `/owner`
- `src/routes/owner_dashboard.rs` — renders owner_chat at `/owner/dashboard`
- `src/components/mod.rs` — exports both owner_login and owner_chat

## Remaining Items
- Chat sub-components could still be extracted as Topcoat components for cleanliness, but not required (JS handles DOM)
- `routes/home.rs` at `/` shows hero section; Astro had everything on one page (`/`). Currently `/` → hero, `/chat` → full chat. Intentional.
- Need to verify with actual running binary that JS modules load + WS connects + auth works
- Release build + asset bundle + Docker deploy not yet done

## YAML Runtime Loading
- landing.yaml read at startup, defaults available if file missing
- Env vars: ZEROWRAPPER_LANDING_YAML, ZEROWRAPPER_SETTINGS_YAML
- Edit YAML → restart binary to pick up changes
