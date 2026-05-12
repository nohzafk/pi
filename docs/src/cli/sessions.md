# Sessions

Every interactive turn is saved to disk under
`$XDG_CONFIG_HOME/pi/sessions/<id>.json` (typically
`~/.config/pi/sessions/` on Linux/macOS). Sessions contain the full
message transcript and can be reloaded later.

## Subcommands

```bash
pi sessions list           # list saved sessions (id, model, last updated)
pi sessions show <id>      # print a session's transcript
pi sessions delete <id>    # delete a saved session
```

## Resume

To continue a previous conversation, pass `--resume <id>`:

```bash
pi --resume 0193abcd-...
```

Inside the REPL you can also use `/resume <id>` to swap to a saved
session, or `/session` to print the current session id.
