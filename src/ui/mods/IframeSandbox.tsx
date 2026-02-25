import { useEffect, useRef, useCallback } from 'react';
import { modMessageBus } from './ModMessageBus';
import { invoke } from '@tauri-apps/api/core';
import { open as dialogOpen } from '@tauri-apps/plugin-dialog';

/** Message protocol between host ↔ iframe */
interface ModMessage {
    type: 'prop-update' | 'event' | 'action' | 'ready' | 'invoke';
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

// ── Blob → Data URL cache ─────────────────────────────
// Converts blob: URLs to base64 data: URLs so cross-origin iframes can render them.
const blobToDataCache = new Map<string, string>();

async function blobUrlToDataUrl(blobUrl: string): Promise<string> {
    if (blobToDataCache.has(blobUrl)) return blobToDataCache.get(blobUrl)!;

    try {
        const response = await fetch(blobUrl);
        const blob = await response.blob();
        return new Promise((resolve, reject) => {
            const reader = new FileReader();
            reader.onloadend = () => {
                const dataUrl = reader.result as string;
                blobToDataCache.set(blobUrl, dataUrl);
                resolve(dataUrl);
            };
            reader.onerror = reject;
            reader.readAsDataURL(blob);
        });
    } catch (e) {
        console.warn(`[IframeSandbox] Failed to convert blob URL: ${blobUrl}`, e);
        return blobUrl; // fallback to original
    }
}

/**
 * Deep-walk an object and convert all blob: URL strings to data: URLs.
 * Returns a new object (never mutates the original).
 */
async function convertBlobUrls(obj: unknown): Promise<unknown> {
    if (typeof obj === 'string') {
        if (obj.startsWith('blob:')) return blobUrlToDataUrl(obj);
        return obj;
    }

    if (Array.isArray(obj)) {
        return Promise.all(obj.map(item => convertBlobUrls(item)));
    }

    if (obj && typeof obj === 'object') {
        const entries = Object.entries(obj as Record<string, unknown>);
        const converted = await Promise.all(
            entries.map(async ([key, val]) => [key, await convertBlobUrls(val)] as const)
        );
        return Object.fromEntries(converted);
    }

    return obj;
}

export const IframeSandbox = ({
    src,
    id,
    permissions: _permissions = [],
    componentProps,
    onModEvent,
}: IframeSandboxProps) => {
    const iframeRef = useRef<HTMLIFrameElement>(null);
    const readyRef = useRef(false);

    // Store volatile callbacks/props in refs to avoid re-triggering the
    // message-listener useEffect (which would unregister & re-register
    // the component on every render).
    const componentPropsRef = useRef(componentProps);
    componentPropsRef.current = componentProps;

    const onModEventRef = useRef(onModEvent);
    onModEventRef.current = onModEvent;

    // Build sandbox attribute — relaxed for better mod DX.
    // allow-same-origin: required for custom protocol loading
    // allow-scripts: required for JS execution
    // allow-forms: allows form submissions inside mod UI
    // allow-modals: allows alert/confirm/prompt dialogs
    // allow-popups: allows window.open (when overlay permission granted)
    // allow-popups + allow-popups-to-escape-sandbox: enables <input type="file"> and Tauri dialogs
    const baseSandbox = 'allow-scripts allow-same-origin allow-forms allow-modals allow-popups allow-popups-to-escape-sandbox';
    const sandboxFlags = baseSandbox;

    // Send props to iframe when they change.
    // postMessage uses the structured clone algorithm which cannot handle
    // functions, symbols, or undefined values. Strip them, then convert
    // blob: URLs to data: URLs so cross-origin iframes can display them.
    const sendToIframe = useCallback(async (msg: ModMessage) => {
        if (!iframeRef.current?.contentWindow || !readyRef.current) return;

        // 1) Strip functions / non-serializable values
        const safe = JSON.parse(JSON.stringify(msg, (_key, value) =>
            typeof value === 'function' ? undefined : value
        ));

        // 2) Convert blob: URLs inside the payload so the iframe can load them
        if (safe.payload) {
            safe.payload = await convertBlobUrls(safe.payload);
        }

        iframeRef.current?.contentWindow?.postMessage(safe, '*');
    }, []);

    // Forward prop updates to iframe
    useEffect(() => {
        if (componentProps) {
            sendToIframe({ type: 'prop-update', payload: componentProps });
        }
    }, [componentProps, sendToIframe]);

    // Listen for messages FROM the iframe.
    // Only depends on `id` — props/callbacks are accessed via refs.
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
                    if (componentPropsRef.current) {
                        sendToIframe({ type: 'prop-update', payload: componentPropsRef.current });
                    }
                    console.log(`[ModFrame ${id}] Component ready`);
                    break;

                case 'event':
                    if (onModEventRef.current && msg.payload && typeof msg.payload === 'object') {
                        const { name, ...rest } = msg.payload as { name: string;[key: string]: unknown };
                        onModEventRef.current(name, rest);
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

                // Mod requests a Tauri invoke — proxy through the host window
                case 'invoke': {
                    const invokePayload = msg.payload as {
                        id?: string;
                        command?: string;
                        args?: Record<string, unknown>;
                    } | undefined;

                    if (invokePayload?.command && invokePayload?.id) {
                        // Route plugin:dialog|open through the dialog API
                        const cmd = invokePayload.command;
                        let promise: Promise<unknown>;

                        if (cmd === 'plugin:dialog|open') {
                            const args = invokePayload.args ?? {};
                            promise = dialogOpen({
                                multiple: args.multiple as boolean ?? false,
                                filters: args.filters as Array<{ name: string; extensions: string[] }> ?? [],
                                title: args.title as string ?? undefined,
                            });
                        } else {
                            promise = invoke(cmd, invokePayload.args ?? {});
                        }

                        promise
                            .then((result) => {
                                iframeRef.current?.contentWindow?.postMessage({
                                    type: 'invoke-result',
                                    payload: { id: invokePayload.id, result },
                                }, '*');
                            })
                            .catch((err) => {
                                iframeRef.current?.contentWindow?.postMessage({
                                    type: 'invoke-result',
                                    payload: { id: invokePayload.id, error: String(err) },
                                }, '*');
                            });
                    }
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
    }, [id, sendToIframe]);

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
