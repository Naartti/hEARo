import Foundation
import Testing

@testable import HearoKit

/// The one-time copy of preferences from the old bundle id (`dev.hearo.app`).
struct DefaultsMigrationTests {
    private func freshDefaults() -> (UserDefaults, String) {
        let name = "hearo-migration-test-\(UUID().uuidString)"
        return (UserDefaults(suiteName: name)!, name)
    }

    @Test func copiesEveryOldKeyOnce() {
        let (defaults, name) = freshDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        let old: [String: Any] = ["autoCapture.enabled": true, "autoCapture.dailyCap": 120, "review.showSpectrogram": false]

        #expect(DefaultsMigration.run(from: old, to: defaults) == 3)
        #expect(defaults.bool(forKey: "autoCapture.enabled"))
        #expect(defaults.integer(forKey: "autoCapture.dailyCap") == 120)
        #expect(defaults.object(forKey: "review.showSpectrogram") as? Bool == false)

        // A second launch copies nothing, even if the user has since changed a value.
        defaults.set(250, forKey: "autoCapture.dailyCap")
        #expect(DefaultsMigration.run(from: old, to: defaults) == 0)
        #expect(defaults.integer(forKey: "autoCapture.dailyCap") == 250)
    }

    @Test func neverOverwritesAValueTheNewAppAlreadyHas() {
        let (defaults, name) = freshDefaults()
        defer { defaults.removePersistentDomain(forName: name) }
        defaults.set(50, forKey: "autoCapture.dailyCap")

        #expect(DefaultsMigration.run(from: ["autoCapture.dailyCap": 120, "running": true], to: defaults) == 1)
        #expect(defaults.integer(forKey: "autoCapture.dailyCap") == 50)
        #expect(defaults.bool(forKey: "running"))
    }

    @Test func nothingToMigrateStillMarksItDone() {
        let (defaults, name) = freshDefaults()
        defer { defaults.removePersistentDomain(forName: name) }

        #expect(DefaultsMigration.run(from: nil, to: defaults) == 0)
        // Old preferences appearing later (another copy of an old build) are not pulled in.
        #expect(DefaultsMigration.run(from: ["running": true], to: defaults) == 0)
        #expect(defaults.object(forKey: "running") == nil)
    }
}
