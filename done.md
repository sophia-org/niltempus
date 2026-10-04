# Completed work

Completed task lines live in monthly todo.txt-format files named
`done-YYYY-MM.md` beside `todo.md`. The CLI writes directly to the current
month; this page stays a short guide. Keeping the task files together
preserves their relative Markdown links when the todo.txt CLI moves a line.
Each line retains its stable task ID, completion date, and the link to the
note holding its result and evidence.

Use `zk completed` to list the files, or `zk list --link-to NOTE_PATH` to find
completion records linked to a particular note. `zk tasks` refreshes the index
after each successful command. Direct edits need `zk index` afterward.
