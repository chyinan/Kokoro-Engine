import { useEffect, useRef, useCallback } from 'react';
import { modMessageBus } from './ModMessageBus';
import { invoke } from '@tauri-apps/api/core';

/** Message protocol between host ↔ iframe */
interface ModMessage {
    type: 'prop-update' | 'event' | 'action' | 'ready';
    payload?: unknown;
}

interface IframeSandboxProps {
    src: string;
    id: string;
    permissions?: string[];
    /** Props to forward to the iframe component via postMessage */
    componentProps?: Record<string, unknown>;
    /** Called when the iframe emits an event */
    onModEvent?: (eventName: string, payload: unknown) => void;
}

export const IframeSandbox = ({
    src,
    id,
    permissions = [],
    componentProps,
    onModEvent,
}: IframeSandboxProps) => {
    const iframeRef = useRef<HTMLIFrameElement>(null);
    const readyRef = useRef(false);

    // Build sandbox attribute based on permissions
    const sandboxFlags = permissions.includes('ui:overlay')
        ? 'allow-scripts allow-popups'
        : 'allow-scripts';

    // Send props to iframe when they change
    const sendToIframe = useCallback((msg: ModMessage) => {
        if (iframeRef.current?.contentWindow && readyRef.current) {
            iframeRef.current.contentWindow.postMessage(msg, '*');
        }
    }, []);

    // Forward prop updates to iframe
    useEffect(() => {
        if (componentProps) {
            sendToIframe({ type: 'prop-update', payload: componentProps });
        }
    }, [componentProps, sendToIframe]);

    // Listen for messages FROM the iframe
    useEffect(() => {
        const handleMessage = (event: MessageEvent<ModMessage>) => {
            if (event.source !== iframeRef.current?.contentWindow) return;

            const msg = event.data;
            if (!msg || typeof msg !== 'object' || !msg.type) return;

            switch (msg.type) {
                case 'ready':
                    readyRef.current = true;
                    // Register this iframe with the message bus
                    if (iframeRef.current?.contentWindow) {
                        modMessageBus.register(id, iframeRef.current.contentWindow);
                    }
                    // Send initial props once iframe is ready
                    if (componentProps) {
                        sendToIframe({ type: 'prop-update', payload: componentProps });
                    }
                    console.log(`[ModFrame ${id}] Component ready`);
                    break;

                case 'event':
                    if (onModEvent && msg.payload && typeof msg.payload === 'object') {
                        const { name, ...rest } = msg.payload as { name: string;[key: string]: unknown };
                        onModEvent(name, rest);
                    }
                    break;

                case 'action': {
                    const actionPayload = msg.payload as { action?: string; data?: unknown } | undefined;
                    console.log(`[ModFrame ${id}] Action:`, actionPayload);

                    // Route actions to the QuickJS script runtime via Tauri
                    if (actionPayload?.action) {
                        invoke('dispatch_mod_event', {
                            event: `action:${actionPayload.action}`,
                            payload: actionPayload.data ?? null,
                        }).catch((err) => {
                            console.error(`[ModFrame ${id}] Failed to dispatch action:`, err);
                        });
                    }

                    // Also emit a DOM CustomEvent so the host React app can react
                    document.dispatchEvent(
                        new CustomEvent('kokoro:mod-action', {
                            detail: { componentId: id, ...actionPayload },
                        })
                    );
                    break;
                }
            }
        };

        window.addEventListener('message', handleMessage);
        return () => {
            window.removeEventListener('message', handleMessage);
            readyRef.current = false;
            // Unregister from message bus on unmount
            modMessageBus.unregister(id);
        };
    }, [id, componentProps, onModEvent, sendToIframe]);

    return (
        <iframe
            ref={iframeRef}
            src={src}
            title={`Mod Component: ${id}`}
            sandbox={sandboxFlags}
            style={{
                border: 'none',
                width: '100%',
                height: '100%',
                background: 'transparent'
            }}
        />
    );
};
