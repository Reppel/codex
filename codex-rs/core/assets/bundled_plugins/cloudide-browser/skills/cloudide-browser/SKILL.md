---
name: cloudide-browser
description: Use the CloudIDE Pod's persistent browser for website tasks that require interactive pages or the user's logged-in browser session.
---

Use the CloudIDE browser MCP for Pod browser tasks. Start with `observe`, perform one action with `interact`, then inspect the returned page state before continuing. Use the same page and session when possible.

If a site presents a security verification or access challenge, keep the page open, call `takeover`, and ask the user to complete it in the live viewer. Call `return_control` after the user confirms completion, then observe the page before resuming. Do not bypass site verification or alter browser identity.
