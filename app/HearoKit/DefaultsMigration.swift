import Foundation

/// One-time copy of preferences from the app's previous bundle id.
///
/// macOS keys `UserDefaults` by bundle id, so the rename from `dev.hearo.app` to
/// `io.github.naartti.hearo` (2026-11-15) would otherwise forget auto-save settings, Review
/// options and window positions. Module settings live in `settings.json` and need no copy.
public enum DefaultsMigration {
    /// The bundle id hEARo used before 2026-11-15.
    public static let previousDomain = "dev.hearo.app"
    static let doneKey = "migratedFrom.dev.hearo.app"

    /// Copies `old` into `defaults` the first time it runs, never overwriting a key `defaults`
    /// already has, then marks itself done (also when there was nothing to copy).
    /// Returns how many keys were copied.
    @discardableResult
    public static func run(from old: [String: Any]?, to defaults: UserDefaults) -> Int {
        guard !defaults.bool(forKey: doneKey) else { return 0 }
        var copied = 0
        for (key, value) in old ?? [:] where defaults.object(forKey: key) == nil {
            defaults.set(value, forKey: key)
            copied += 1
        }
        defaults.set(true, forKey: doneKey)
        return copied
    }

    /// Runs the migration for the installed app (reads the old domain from the user's preferences).
    public static func runForApp() {
        let defaults = UserDefaults.standard
        run(from: defaults.persistentDomain(forName: previousDomain), to: defaults)
    }
}
