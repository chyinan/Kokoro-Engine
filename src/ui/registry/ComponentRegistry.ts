import { ComponentType } from "react";
import { IframeSandbox } from "../mods/IframeSandbox";

// Props are unknown at registration time; consumers cast as needed
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type ComponentConstructor = ComponentType<any>;

interface ModComponentEntry {
    modId: string;
    src: string; // mod:// URL
}

export class ComponentRegistry {
    private components = new Map<string, ComponentConstructor>();
    private modComponents = new Map<string, ModComponentEntry>();
    private listeners = new Set<() => void>();

    /**
     * Register a core component. Will NOT overwrite a mod-registered
     * component so that HMR / page reload doesn't clobber mod overrides.
     */
    register(name: string, component: ComponentConstructor) {
        if (this.modComponents.has(name)) return; // mod takes precedence
        this.components.set(name, component);
        this.notify();
    }

    /**
     * Register a mod-provided iframe component as a slot override.
     * When LayoutRenderer looks up this name, it gets a React component
     * that renders the mod's HTML inside an IframeSandbox.
     */
    registerModComponent(slotName: string, modId: string, src: string) {
        this.modComponents.set(slotName, { modId, src });

        // Tauri v2: custom protocols are accessed via http://<scheme>.localhost/
        // The backend emits mod://modId/path, but iframes need http://mod.localhost/modId/path
        const iframeSrc = src.replace(/^mod:\/\//, 'http://mod.localhost/');
        console.log(`[Registry] registerModComponent '${slotName}' → ${iframeSrc}`);

        // Create a wrapper React component for this mod iframe
        const ModWrapper: ComponentConstructor = (props: Record<string, unknown>) => {
            return IframeSandbox({
                id: `${modId}-${slotName}`,
                src: iframeSrc,
                permissions: [],
                componentProps: props,
            });
        };
        ModWrapper.displayName = `ModComponent(${modId}/${slotName})`;

        this.components.set(slotName, ModWrapper);
        this.notify();
    }

    /**
     * Unregister all components from a specific mod.
     */
    unregisterMod(modId: string) {
        for (const [slot, entry] of this.modComponents.entries()) {
            if (entry.modId === modId) {
                this.modComponents.delete(slot);
                this.components.delete(slot);
            }
        }
        this.notify();
    }

    get(name: string): ComponentConstructor | undefined {
        return this.components.get(name);
    }

    isModComponent(name: string): boolean {
        return this.modComponents.has(name);
    }

    /** Return all registered component names. */
    list(): string[] {
        return Array.from(this.components.keys());
    }

    subscribe(listener: () => void) {
        this.listeners.add(listener);
        return () => this.listeners.delete(listener);
    }

    private notify() {
        this.listeners.forEach((l) => l());
    }
}

export const registry = new ComponentRegistry();
