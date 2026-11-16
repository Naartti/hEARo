# ml/data

Raw and processed datasets live here and are **git-ignored**. Only manifests (with checksums)
and `SOURCES.md` (licenses and attribution) are committed. Populated in M7.

- `processed/snapshots/` — `just ml-import-snapshots`: the human's reviewed snapshots as a
  manifest + mixing pool (docs/ml/README.md). Never committed: the clips hold other people's
  voices, and the manifest points at them by absolute path.
