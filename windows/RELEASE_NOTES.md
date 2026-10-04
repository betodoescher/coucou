## ✨ What's new

Mochi now **keeps an eye on your Claude plan limits and your GitHub CI**, so you know when to slow down and when your PR is ready to merge.

> [!IMPORTANT]
> Open **Settings** and **reinstall the Claude Code hooks** to see your plan limits (and the Cursor hooks too if you are coming from 0.1.x). As always, you see the diff and a backup is made first.

### ⏱️ Claude's plan limits on the home card
- **See how much of your 5-hour and weekly Claude limits you have used**, right next to today's tokens. Hover for both windows and when each one resets.
- Read straight from Claude Code, no extra login. Your own status line keeps working exactly as before: Coucou wraps it and gives it back untouched when you uninstall the hooks.

### 🐙 Your GitHub PRs and their CI
- **The GitHub card now lists your open PRs with a CI dot:** red when it failed, amber while it runs, green when it passed. Failing CI comes first, then reviews waiting for you.
- **Know the moment it matters:** a CI that just failed, a broken default branch or a new review request opens the island. **A CI that turns green** badges the pill with a little chime.
- **Your contribution grid:** the last 7 days sit in the card's header. Click them to see the past 23 weeks, and hover a square for that day's count.
- Fewer requests to GitHub: one query now replaces the per-repository checks.

### 🔒 Still private by design
No telemetry, no account. Keys stay in your system keychain, and the app only talks to the services you turn on.
