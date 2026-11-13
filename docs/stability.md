# What stays stable

From 1.0, Lucy follows [semantic versioning](https://semver.org) for the two things you build on:

- **config.toml:** every setting that works in 1.0 keeps working, and means the same thing, in
  every 1.x release. New settings can appear; their defaults keep today's behaviour unless the
  release notes say otherwise.
- **The `lucy` command:** its subcommands, flags, exit codes, and the output of `lucy ask` that
  scripts read keep working through 1.x.

Something on its way out is **deprecated first**: it keeps working, with a warning saying what
to use instead, for at least one minor release (e.g. deprecated in 1.3, gone no earlier than
2.0). Breaking changes only come in a major version, listed in the [changelog](https://github.com/AJMerr/lucy/blob/main/CHANGELOG.md).

## What each number means

- **Major (2.0.0):** something above stops working as it did. A setting is removed, renamed or
  means something new; a value that used to be accepted is refused; a subcommand or flag is
  removed; exit codes or `lucy ask`'s output change; `memory.md` stops being plain Markdown; a
  sprite pack format or a supported platform is dropped.
- **Minor (1.1.0):** new features, settings, commands and flags; changes to the core's protocol;
  deprecations; and new defaults, when a setting brings the old behaviour back.
- **Patch (1.0.1):** fixes only. No new settings and no protocol change, so `lucy`, the app and
  the core from any two patch releases of the same minor work together.

## Security fixes

Closing a security hole comes before compatibility. A security fix ships in the next release,
a patch if it needs no protocol change, even when it changes behaviour you might rely on (more
calls ask first, say). The changelog lists those changes under **Breaking for security**, with
what to do, and a setting to restore the old behaviour where turning the protection off is a
reasonable choice. Fixed vulnerabilities are published as
[security advisories](https://github.com/AJMerr/lucy/security/advisories); see
[SECURITY.md](https://github.com/AJMerr/lucy/blob/main/SECURITY.md) to report one.

## Not covered

- **The core's protocol** (the JSON-RPC that `lucy`, the app and other front ends speak to the
  core) can change in a minor release. It has its own version, and the core refuses a client
  that's too old or too new, saying so. Use the same version of `lucy`, the app and the core.
- **How she phrases things:** the persona, her replies, and text meant for people (messages,
  the wording of settings and errors) can change at any time.
- **Files in the data folder** (the chat database, caches) are Lucy's own. They're upgraded
  automatically and may not open in an older version. `memory.md` stays plain Markdown.
- **Sprite packs** keep their `format_version`; a new format would get a new number, and old
  packs keep working.
