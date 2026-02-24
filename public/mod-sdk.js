/**
 * Kokoro Mod SDK — Injected into MOD component iframes.
 * Provides the `Kokoro` global API for communication with the host engine.
 * 
 * Usage inside a MOD component HTML:
 *   Kokoro.on("update", (props) => { ... });
 *   Kokoro.emit("action", { target: "jump" });
 *   const result = await Kokoro.invoke("list_mods", {});
 */
(function () {
    "use strict";

    const listeners = {};
    let _ready = false;
    let _invokeIdCounter = 0;
    const _invokePending = new Map();

    const Kokoro = {
        /**
         * Register a listener for events from the host engine.
         * @param {string} eventName - e.g. "update", "theme-change"
         * @param {function} callback
         */
        on(eventName, callback) {
            if (!listeners[eventName]) {
                listeners[eventName] = [];
            }
            listeners[eventName].push(callback);
        },

        /**
         * Remove a listener.
         * @param {string} eventName
         * @param {function} callback
         */
        off(eventName, callback) {
            if (listeners[eventName]) {
                listeners[eventName] = listeners[eventName].filter(
                    (cb) => cb !== callback
                );
            }
        },

        /**
         * Emit an event to the host engine.
         * @param {string} eventName
         * @param {*} payload
         */
        emit(eventName, payload) {
            window.parent.postMessage(
                {
                    type: "event",
                    payload: { name: eventName, ...payload },
                },
                "*"
            );
        },

        /**
         * Emit an action to the host engine.
         * @param {string} actionName
         * @param {*} data
         */
        action(actionName, data) {
            window.parent.postMessage(
                {
                    type: "action",
                    payload: { action: actionName, data },
                },
                "*"
            );
        },

        /**
         * Invoke a Tauri command through the host window proxy.
         * Returns a Promise that resolves with the command result.
         * @param {string} command - Tauri command name
         * @param {object} args - Command arguments
         * @returns {Promise<*>}
         */
        invoke(command, args = {}) {
            return new Promise((resolve, reject) => {
                const id = `inv_${++_invokeIdCounter}`;
                _invokePending.set(id, { resolve, reject });

                window.parent.postMessage(
                    {
                        type: "invoke",
                        payload: { id, command, args },
                    },
                    "*"
                );

                // Timeout after 30s
                setTimeout(() => {
                    if (_invokePending.has(id)) {
                        _invokePending.delete(id);
                        reject(new Error(`Invoke '${command}' timed out`));
                    }
                }, 30000);
            });
        },

        /**
         * Log a message via the host engine console.
         * @param {...*} args
         */
        log(...args) {
            console.log("[ModSDK]", ...args);
            window.parent.postMessage(
                {
                    type: "event",
                    payload: { name: "__log", message: args.join(" ") },
                },
                "*"
            );
        },
    };

    // ── Listen for messages FROM the host engine ──
    window.addEventListener("message", (event) => {
        const msg = event.data;
        if (!msg || typeof msg !== "object" || !msg.type) return;

        switch (msg.type) {
            case "prop-update": {
                // Dispatch to "update" listeners
                const cbs = listeners["update"] || [];
                for (const cb of cbs) {
                    try {
                        cb(msg.payload);
                    } catch (e) {
                        console.error("[ModSDK] Error in update handler:", e);
                    }
                }
                break;
            }
            case "event": {
                // Dispatch to named event listeners
                const payload = msg.payload || {};
                const name = payload.name;
                if (name && listeners[name]) {
                    for (const cb of listeners[name]) {
                        try {
                            cb(payload);
                        } catch (e) {
                            console.error(
                                `[ModSDK] Error in '${name}' handler:`,
                                e
                            );
                        }
                    }
                }
                break;
            }
            case "invoke-result": {
                // Resolve/reject a pending Kokoro.invoke() call
                const { id, result, error } = msg.payload || {};
                if (id && _invokePending.has(id)) {
                    const { resolve, reject } = _invokePending.get(id);
                    _invokePending.delete(id);
                    if (error) {
                        reject(new Error(error));
                    } else {
                        resolve(result);
                    }
                }
                break;
            }
        }
    });

    // ── Signal readiness to the host ──
    window.parent.postMessage({ type: "ready" }, "*");
    _ready = true;

    // Expose globally
    window.Kokoro = Kokoro;
})();
