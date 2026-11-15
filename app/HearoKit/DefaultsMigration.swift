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

    /// Copies `old` into `defaults` (whose own saved values live in the persistent domain
    /// `domain`) the first time it runs, never overwriting a value the app has saved itself,
    /// then marks itself done (also when there was nothing to copy). Everything is copied,
    /// window and status-item positions included; the old domain is left in place so an older
    /// build still finds its settings. Returns how many keys were copied.
    @discardableResult
    public static func run(from old: [String: Any]?, to defaults: UserDefaults, domain: String) -> Int {
        guard !defaults.bool(forKey: doneKey) else { return 0 }
        // Not `defaults.object(forKey:)`: it also sees registered and global (NSGlobalDomain)
        // values, which would wrongly count as "already set".
        let own = defaults.persistentDomain(forName: domain) ?? [:]
        var copied = 0
        for (key, value) in old ?? [:] where own[key] == nil {
            defaults.set(value, forKey: key)
            copied += 1
        }
        defaults.set(true, forKey: doneKey)
        return copied
    }

    /// Runs the migration for the installed app (reads the old domain from the user's preferences).
    public static func runForApp() {
        guard let domain = Bundle.main.bundleIdentifier else { return }
        let defaults = UserDefaults.standard
        run(from: defaults.persistentDomain(forName: previousDomain), to: defaults, domain: domain)
    }
}
