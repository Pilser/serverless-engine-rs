export const els = {}

export function init() {
  els.form = document.getElementById('chat-form')
  els.input = document.getElementById('chat-input')
  els.messages = document.getElementById('chat-messages')
  els.hint = document.getElementById('fallback-hint')
  els.hintText = document.getElementById('chat-hint-text')
  els.chatRefreshBtn = document.getElementById('chat-refresh-btn')
  els.typingEl = document.getElementById('typing-indicator')
  els.authSection = document.getElementById('auth-section')
  els.userBadge = document.getElementById('user-badge')
  els.badgeDisplay = document.getElementById('badge-display')
  els.badgeSub = document.getElementById('badge-sub')
  els.registerBtn = document.getElementById('register-btn')
  els.emailInput = document.getElementById('email-input')
  els.authError = document.getElementById('auth-error')
  els.logoutBtn = document.getElementById('logout-btn')
  els.sendBtn = document.getElementById('send-btn')
  els.cancelBtn = document.getElementById('cancel-btn')
  els.uploadBtn = document.getElementById('upload-btn')
  els.fileInput = document.getElementById('file-input')
  els.attachments = document.getElementById('attachments')
}
