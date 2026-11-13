# Security

Lucy runs commands, edits files and holds API keys for you, so security reports matter a lot.

## Reporting a problem

Please **don't open a public issue**. Report it privately through GitHub instead:
[Security → Report a vulnerability](https://github.com/AJMerr/lucy/security/advisories/new).

Say what you found, how to reproduce it, and what someone could do with it. Lucy has one
maintainer, who aims to reply within a week. Once it's fixed and released, the advisory is
published with credit to you, unless you'd rather not be named.

## Supported versions

Fixes go into the latest release. Please check a problem still happens there before reporting.

A security fix ships in the next release even if it changes behaviour; the changelog lists
those changes under **Breaking for security**. See
[What stays stable](docs/stability.md#security-fixes).

## What counts

For example:

- A tool call that runs, writes or goes online **without the approval** its level and rules
  require, or a way round the private-path checks (`~/.ssh`, `.env` and so on).
- **Keys leaking:** an API key or token ending up in a file, a log, a chat, an error message or
  anything sent to a model.
- **Remote access:** reaching the core without its token, or a remote client getting past
  `[remote]` limits (its maximum level, settings and keys).
- Memory saving something it refuses to (secrets), or the approval card being skipped for
  sensitive facts.
- Anything in the installers, the release workflow or the update path that could ship someone
  else's code.

Not a vulnerability: a model saying something wrong or unpleasant (please open an issue), and
things you've allowed (level 3 runs everything, by design).
