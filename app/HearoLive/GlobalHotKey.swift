import Carbon.HIToolbox
import Foundation

/// A global hotkey via Carbon `RegisterEventHotKey` — needs no Accessibility permission (same
/// mechanism as the KeyboardShortcuts library). While registered, the key is captured
/// system-wide. Default for "Missed it": the § key with no modifiers (docs/DESIGN.md).
@MainActor
public final class GlobalHotKey {
    public static let sectionKey = UInt32(kVK_ISO_Section)

    private var ref: EventHotKeyRef?
    private var handler: EventHandlerRef?
    private let action: @MainActor () -> Void
    private static var instances: [UInt32: GlobalHotKey] = [:]
    private static var nextID: UInt32 = 1
    private static var installed = false

    public init?(keyCode: UInt32 = GlobalHotKey.sectionKey, modifiers: UInt32 = 0, action: @escaping @MainActor () -> Void) {
        self.action = action
        GlobalHotKey.installHandlerOnce()
        let id = GlobalHotKey.nextID
        GlobalHotKey.nextID += 1
        let hotKeyID = EventHotKeyID(signature: OSType(0x6845_4152) /* 'hEAR' */, id: id)
        let status = RegisterEventHotKey(keyCode, modifiers, hotKeyID, GetApplicationEventTarget(), 0, &ref)
        guard status == noErr else { return nil }
        GlobalHotKey.instances[id] = self
    }

    isolated deinit {
        if let ref { UnregisterEventHotKey(ref) }
    }

    private static func installHandlerOnce() {
        guard !installed else { return }
        installed = true
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetApplicationEventTarget(), { _, event, _ in
            var id = EventHotKeyID()
            GetEventParameter(event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID),
                              nil, MemoryLayout<EventHotKeyID>.size, nil, &id)
            let hotKeyNumber = id.id
            DispatchQueue.main.async {
                MainActor.assumeIsolated { GlobalHotKey.instances[hotKeyNumber]?.action() }
            }
            return noErr
        }, 1, &spec, nil, nil)
    }
}
