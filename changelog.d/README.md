# changelog.d — pending changelog entries

One file per pull request with a change a user would notice, named after the branch with `/`
as `-` (`fix/desmack-cluster` → `fix-desmack-cluster.md`). Feature PRs never edit
[CHANGELOG.md](../CHANGELOG.md) itself: every PR inserting lines under the same *Unreleased*
heading made every parallel PR conflict (2026-11-16). A new file per PR can't conflict.

```markdown
### Fixed

- De-smack: a loud smack no longer raises the bar for a quieter one right after it.
```

Headings are Keep a Changelog's: *Added*, *Changed*, *Deprecated*, *Removed*, *Fixed*,
*Security*. Write for the user, one bullet per change. A later PR that changes the same
behaviour before a release edits that PR's fragment.

- `just changelog` previews the next release's section.
- `just check` rejects a malformed fragment, and a feature branch that edits CHANGELOG.md or
  docs/PROGRESS.md.
- The release folds the fragments into CHANGELOG.md and deletes them
  ([docs/dev/RELEASING.md](../docs/dev/RELEASING.md)).
