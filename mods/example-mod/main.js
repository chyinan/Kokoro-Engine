// Example MOD script — demonstrates the Kokoro API
Kokoro.log("Hello from Mod! The system is working.");
Kokoro.log("Engine version: " + Kokoro.version);

// Register event listeners
Kokoro.on("chat", function (msg) {
    Kokoro.log("Chat received in mod!");
});

// Emit a custom event
Kokoro.emit("mod-loaded", { name: "test-mod" });
