# Security policy

## Supported versions

hEARo is pre-1.0. Only the latest commit on the default branch is supported; fixes are not
backported.

## Reporting a vulnerability

**Please do not report security problems in public issues, pull requests or discussions.**

Report them privately through GitHub's **private vulnerability reporting**: open the
repository's **Security** tab and choose **Report a vulnerability**. Only the maintainers can
see the report. Please include:

- what the problem is and what an attacker could do with it,
- steps to reproduce (macOS version, Mac model, hEARo commit),
- any proof of concept, logs or crash reports — but **no audio recordings of other people**.

You can expect an acknowledgement within a week. We'll keep you updated, agree on a disclosure
date with you, and credit you in the release notes unless you prefer otherwise.

## What is in scope

hEARo processes all system audio and holds the macOS **System Audio Recording** permission,
so issues we especially want to hear about include:

- any way audio could leave the Mac, be recorded without the user saving a snapshot, or be
  read by another process through hEARo;
- snapshot files (`~/Library/Application Support/hEARo/snapshots/`) being written, exported or
  imported unsafely (e.g. path traversal in snapshot import);
- memory-safety bugs in the Rust engine or the Swift ↔ Rust bridge;
- weaknesses in how the app is signed or how the build scripts handle secrets.

hEARo contains no network code by design; a report showing otherwise is in scope too.
