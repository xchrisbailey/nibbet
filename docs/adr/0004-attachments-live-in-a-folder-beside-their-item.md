---
status: accepted
---

# Attachments live in a folder beside their Item

An Item's Attachments are copied into a folder named after its slug, next to its file: `tasks/fix-login.md` keeps them in `tasks/fix-login/`, and `journal/2026-11-14.md` in `journal/2026-11-14/`. The body links them with ordinary relative Markdown, `![alt](fix-login/screenshot.png)` or `[report.pdf](fix-login/report.pdf)`, so any Markdown tool or agent can open them (ADR 0001). The folder is created on the first Attachment.

We chose this over one `attachments/` folder per Space and over one per directory. Slugs never change and Tasks and Notes never change folders, so the only event an Attachment has to follow is its Item going to the Trash. A folder that belongs to a single Item goes along with it, with no reference counting and no orphans to clean up. Links stay one segment deep wherever the Item lives. A shared Space folder would need `../../../attachments/x.png` from a Project's Task, which agents get wrong easily.

## Consequences

- Trashing an Item also trashes its folder. Trashing a Project already takes every folder inside it.
- An image used in two Items is stored twice.
- A Template can be renamed, so renaming one renames its folder and rewrites its links. Using a Template has to copy its Attachments into the new Item's folder.
- Folders in `tasks/`, `notes/`, `journal/` and `templates/` are no longer always Projects or junk. The store and watcher already list only `.md` files there.
