/**
 * ModMessageBus — Singleton for routing messages between Kokoro host and MOD iframes.
 *
 * Tracks registered iframe windows by component name and provides
 * targeted send / broadcast capabilities.
 */

export interface ModBusMessage {
    type: "prop-update" | "event";
    payload?: unknown;
}

class ModMessageBus {
    /** Map of component slot name → iframe contentWindow */
    private windows = new Map<string, Window>();

    /** Register an iframe's contentWindow for a given component name. */
    register(name: string, win: Window) {
        this.windows.set(name, win);
        console.log(`[ModMessageBus] Registered component '${name}'`);
    }

    /** Unregister a component by name. */
    unregister(name: string) {
        this.windows.delete(name);
        console.log(`[ModMessageBus] Unregistered component '${name}'`);
    }

    /** Send a message to a specific component iframe. */
    send(name: string, message: ModBusMessage) {
        const win = this.windows.get(name);
        if (win) {
            win.postMessage(message, "*");
        } else {
            console.warn(`[ModMessageBus] No iframe registered for component '${name}'`);
        }
    }

    /** Broadcast a message to all registered mod iframes. */
    broadcast(message: ModBusMessage) {
        for (const win of this.windows.values()) {
            win.postMessage(message, "*");
        }
    }

    /** Check if a component is registered. */
    has(name: string): boolean {
        return this.windows.has(name);
    }
}

export const modMessageBus = new ModMessageBus();
