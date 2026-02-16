// Demo Echo — QuickJS Script
// Listens for engine events and sends data to the DemoPanel component.

Kokoro.log("Demo Echo mod loaded");

// React to chat events — forward a summary to the DemoPanel UI component
Kokoro.on("chat", function (msg) {
    Kokoro.log("Chat received: " + (msg.text || ""));
    Kokoro.ui.send("DemoPanel", { type: "chat-echo", text: msg.text || "" });
});

// React to action events coming FROM the mod UI (via dispatch_mod_event)
Kokoro.on("action:ping", function (data) {
    Kokoro.log("Ping received from DemoPanel! Timestamp: " + (data && data.timestamp ? data.timestamp : "unknown"));
    Kokoro.ui.send("DemoPanel", { type: "pong", timestamp: Date.now() });
});
