import { describe, it, expect, vi, beforeEach } from 'vitest';
import { modMessageBus } from './ModMessageBus';

// Create a mock Window-like object with postMessage
function createMockWindow(): Window {
    return { postMessage: vi.fn() } as unknown as Window;
}

describe('ModMessageBus', () => {
    beforeEach(() => {
        // Clear all registrations between tests
        // We access the private map via the public API
        // Unregister any lingering names
        ['a', 'b', 'c', 'TestPanel', 'DemoPanel', 'Unknown'].forEach(name => {
            modMessageBus.unregister(name);
        });
    });

    it('should register and send to a specific component', () => {
        const win = createMockWindow();
        modMessageBus.register('TestPanel', win);

        modMessageBus.send('TestPanel', { type: 'event', payload: { name: 'test' } });

        expect(win.postMessage).toHaveBeenCalledWith(
            { type: 'event', payload: { name: 'test' } },
            '*'
        );
    });

    it('should not throw when sending to an unregistered component', () => {
        expect(() => {
            modMessageBus.send('NonExistent', { type: 'event', payload: {} });
        }).not.toThrow();
    });

    it('should unregister a component', () => {
        const win = createMockWindow();
        modMessageBus.register('TestPanel', win);
        modMessageBus.unregister('TestPanel');

        modMessageBus.send('TestPanel', { type: 'event', payload: {} });

        expect(win.postMessage).not.toHaveBeenCalled();
    });

    it('should broadcast to all registered windows', () => {
        const winA = createMockWindow();
        const winB = createMockWindow();
        modMessageBus.register('a', winA);
        modMessageBus.register('b', winB);

        const msg = { type: 'event' as const, payload: { name: 'broadcast-test' } };
        modMessageBus.broadcast(msg);

        expect(winA.postMessage).toHaveBeenCalledWith(msg, '*');
        expect(winB.postMessage).toHaveBeenCalledWith(msg, '*');
    });

    it('should not broadcast to unregistered windows', () => {
        const winA = createMockWindow();
        const winB = createMockWindow();
        modMessageBus.register('a', winA);
        modMessageBus.register('b', winB);
        modMessageBus.unregister('a');

        modMessageBus.broadcast({ type: 'event', payload: {} });

        expect(winA.postMessage).not.toHaveBeenCalled();
        expect(winB.postMessage).toHaveBeenCalled();
    });

    it('should report has() correctly', () => {
        const win = createMockWindow();
        expect(modMessageBus.has('DemoPanel')).toBe(false);

        modMessageBus.register('DemoPanel', win);
        expect(modMessageBus.has('DemoPanel')).toBe(true);

        modMessageBus.unregister('DemoPanel');
        expect(modMessageBus.has('DemoPanel')).toBe(false);
    });

    it('should replace registration on duplicate register', () => {
        const win1 = createMockWindow();
        const win2 = createMockWindow();
        modMessageBus.register('c', win1);
        modMessageBus.register('c', win2);

        modMessageBus.send('c', { type: 'event', payload: {} });

        expect(win1.postMessage).not.toHaveBeenCalled();
        expect(win2.postMessage).toHaveBeenCalled();
    });
});
