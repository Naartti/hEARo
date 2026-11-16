### Changed

- Snapshots now record which modules were on and their settings, and `hearo dataset eval`
  replays the modules that ran before the scored one with those settings (older snapshots run
  them at their defaults, and the report says how many).
