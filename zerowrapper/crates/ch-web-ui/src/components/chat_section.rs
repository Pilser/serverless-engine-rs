use topcoat::Result;
use topcoat::view::{view, component, Unescaped};
use topcoat::context::{Cx, app_context};

use crate::context::AppContext;

#[component]
pub async fn chat_section(cx: &Cx) -> Result {
    let ctx: &AppContext = app_context(cx);

    view! {
        <section class="px-6 py-8 md:px-10 mb-2 md:mb-4" style="border-top: 1px solid var(--color-border-weak);">
            <div id="chat-messages">
                <div id="fallback-hint" class="flex flex-col items-center justify-center py-12 gap-3">
                    <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="var(--color-text-weak)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" style="opacity: 0.5;"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>
                    <p id="chat-hint-text" class="text-center leading-relaxed" style="color: var(--color-text-weak); font-size: 0.8125rem;" data-fallback=(&ctx.landing.hero.fallback)>"Connecting to backend…"</p>
                    <button id="chat-refresh-btn" class="hidden flex items-center gap-1.5 px-3 py-1.5 rounded text-xs cursor-pointer transition" style="color: var(--color-text-weak); border: 1px solid var(--color-border-weak); background: var(--color-bg-weak);" type="button">
                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
                        "Refresh"
                    </button>
                </div>
            </div>
            <div id="typing-indicator" class="hidden" style="padding: 12px 16px 12px 52px; margin-bottom: 10px; position: relative;">
                <div class="flex gap-1.5 items-center">
                    <span class="w-3 h-3 rounded-full" style="background: var(--color-text-weak); animation: chat-typing-bounce 1.4s infinite ease-in-out both;"></span>
                    <span class="w-3 h-3 rounded-full" style="background: var(--color-text-weak); animation: chat-typing-bounce 1.4s infinite ease-in-out both; animation-delay: 0.16s;"></span>
                    <span class="w-3 h-3 rounded-full" style="background: var(--color-text-weak); animation: chat-typing-bounce 1.4s infinite ease-in-out both; animation-delay: 0.32s;"></span>
                </div>
            </div>
            <div id="chat-bottom-bar" style="position: sticky; bottom: 0; z-index: 10; background: var(--color-bg); padding-top: 12px; margin-top: 8px;">
                <div id="auth-section">
                    <div id="auth-form" class="flex flex-col gap-3 mt-6 p-5 rounded-xl" style="border: 1px solid var(--color-border); background: var(--color-bg);">
                        <div class="flex items-center gap-3 mb-1">
                            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="var(--color-bg-interactive)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z"/></svg>
                            <span class="text-sm font-bold" style="color: var(--color-text-strong);">"Enter your email to start"</span>
                        </div>
                        <input id="email-input" type="email" placeholder="Your email" class="w-full px-4 py-2.5 text-sm rounded-lg outline-none transition" style="border: 1.5px solid var(--color-border); background: var(--color-bg-weak); color: var(--color-text-strong); font-family: var(--font-mono);"/>
                        <button id="register-btn" class="w-full px-6 py-2.5 text-sm font-medium rounded-lg cursor-pointer transition" style="background: var(--color-bg-interactive); color: var(--color-text-inverted); border: none;">"Start Chat"</button>
                        <p id="auth-error" class="text-xs hidden" style="color: #ff3b30; text-align: center;"></p>
                    </div>
                </div>
                <div id="user-badge" class="hidden flex items-center gap-2 mt-4 px-3 py-1.5 rounded-lg" style="border: 1px solid var(--color-border-weak); background: var(--color-bg-weak);">
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--color-text-weak)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/></svg>
                    <span id="badge-display" class="text-xs flex-1" style="color: var(--color-text-strong);"></span>
                    <span id="badge-sub" class="text-xs" style="color: var(--color-text-weak);"></span>
                    <button id="logout-btn" class="text-xs px-2 py-0.5 rounded cursor-pointer transition" style="color: var(--color-text-weak); border: 1px solid var(--color-border-weak); background: transparent;">"Logout"</button>
                </div>
                <div style="padding: 8px 16px 4px;">
                    <div id="attachments" class="flex flex-wrap gap-1 mb-1"></div>
                </div>
                <form id="chat-form" style="padding-top: 8px;">
                    <div class="flex gap-2 items-end">
                        <button type="button" id="upload-btn" class="flex items-center justify-center cursor-pointer transition rounded-lg flex-shrink-0" style="width: 36px; height: 36px; border: 1.5px solid var(--color-border); background: var(--color-bg-weak); color: var(--color-text-weak); margin-bottom: 2px;" title="Upload file">
                            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>
                        </button>
                        <input type="file" id="file-input" class="hidden" multiple="true" />
                        <div class="flex-1 relative">
                            <textarea id="chat-input" placeholder="Type your message here..." rows="2" class="w-full px-5 py-3.5 pr-12 text-sm rounded-xl resize-none outline-none transition leading-relaxed" style="min-height: 52px; max-height: 160px; border: 1.5px solid var(--color-border); background: var(--color-bg-weak); color: var(--color-text-strong); font-family: var(--font-mono);"></textarea>
                            <button id="send-btn" class="absolute bottom-2 right-2 z-10 flex items-center justify-center cursor-pointer transition rounded-lg" style="width: 32px; height: 32px; border: none; background: var(--color-bg-interactive); color: var(--color-text-inverted);">
                                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/></svg>
                            </button>
                            <button id="cancel-btn" class="absolute bottom-2 right-2 z-10 hidden items-center justify-center cursor-pointer transition rounded-lg" style="width: 32px; height: 32px; border: none; background: var(--color-status-error); color: #fff;">
                                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/></svg>
                            </button>
                        </div>
                    </div>
                </form>
            </div>
        </section>
        <style>{ (Unescaped::new_unchecked(r#"@keyframes chat-typing-bounce { 0%, 80%, 100% { transform: scale(0); } 40% { transform: scale(1); } }
#email-input:focus { border-color: var(--color-bg-interactive) !important; box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-bg-interactive) 25%, transparent) !important; }
#email-input::placeholder { color: var(--color-text-weak); opacity: 0.7; }
#register-btn:hover { opacity: 0.85; }"#)) }</style>
    }
}