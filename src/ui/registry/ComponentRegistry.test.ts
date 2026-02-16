import { describe, it, expect, vi, beforeEach } from 'vitest';
import { ComponentRegistry } from './ComponentRegistry';

// Mock IframeSandbox since it imports React/Tauri
vi.mock('../mods/IframeSandbox', () => ({
    IframeSandbox: vi.fn((props: Record<string, unknown>) => {
        // Return a mock element descriptor
        return { type: 'iframe', props };
    }),
}));

describe('ComponentRegistry', () => {
    let registry: ComponentRegistry;

    beforeEach(() => {
        registry = new ComponentRegistry();
    });

    it('should register a core component', () => {
        const MockComponent = () => null;
        registry.register('ChatBubble', MockComponent);

        expect(registry.get('ChatBubble')).toBe(MockComponent);
    });

    it('should return undefined for unregistered component', () => {
        expect(registry.get('NonExistent')).toBeUndefined();
    });

    it('should register a mod component', () => {
        registry.registerModComponent('DemoPanel', 'demo-echo', 'mod://demo-echo/components/DemoPanel.html');

        // Should be in the main components map
        const component = registry.get('DemoPanel');
        expect(component).toBeDefined();
        expect(typeof component).toBe('function');
    });

    it('should track mod components via isModComponent', () => {
        registry.registerModComponent('DemoPanel', 'demo-echo', 'mod://demo-echo/components/DemoPanel.html');

        expect(registry.isModComponent('DemoPanel')).toBe(true);
        expect(registry.isModComponent('ChatBubble')).toBe(false);
    });

    it('should call subscriber on registration', () => {
        const subscriber = vi.fn();
        registry.subscribe(subscriber);

        registry.register('TestComponent', () => null);

        expect(subscriber).toHaveBeenCalled();
    });

    it('should call subscriber on mod component registration', () => {
        const subscriber = vi.fn();
        registry.subscribe(subscriber);

        registry.registerModComponent('ModPanel', 'test-mod', 'mod://test-mod/panel.html');

        expect(subscriber).toHaveBeenCalled();
    });

    it('should unsubscribe correctly', () => {
        const subscriber = vi.fn();
        const unsubscribe = registry.subscribe(subscriber);
        unsubscribe();

        registry.register('TestComponent', () => null);

        expect(subscriber).not.toHaveBeenCalled();
    });

    it('should set displayName on mod wrapper', () => {
        registry.registerModComponent('DemoPanel', 'demo-echo', 'mod://demo-echo/components/DemoPanel.html');

        const component = registry.get('DemoPanel');
        expect(component?.displayName).toBe('ModComponent(demo-echo/DemoPanel)');
    });

    it('should list all registered component names', () => {
        registry.register('A', () => null);
        registry.register('B', () => null);
        registry.registerModComponent('C', 'mod-x', 'mod://mod-x/c.html');

        const names = registry.list();
        expect(names).toContain('A');
        expect(names).toContain('B');
        expect(names).toContain('C');
    });
});
