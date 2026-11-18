import { commands, events } from "./generated/ipc";

// Compiled by the project's tsc check. These invalid calls must stay invalid.
export function checkIpcTypes() {
    // @ts-expect-error Command names come from Rust registration.
    void commands.getBogusEngineInfo();
    // @ts-expect-error Window dimensions must be numbers.
    void commands.resizePetWindow("320", 240);
    // @ts-expect-error The request must include a message.
    void commands.streamChat({ client_request_id: "request-id" });
    // @ts-expect-error This event carries a string, matching the Rust emitter.
    void events.bubbleTextUpdate.emit({ text: "hello" });
    // @ts-expect-error The reply is engine metadata.
    const name: Promise<number> = commands.getEngineInfo();
    return name;
}

export async function checkSerdeDirections() {
    // Rust supplies defaults for omitted input fields.
    await commands.saveLlmConfig({ providers: [{ id: "local", provider_type: "ollama" }] });
    const config = await commands.getLlmConfig();
    const enabled: boolean = config.providers[0].enabled;
    // @ts-expect-error The serialized config always includes the provider list.
    config.providers = undefined;
    void events.chatRuntimeChanged.listen(({ payload }) => {
        // @ts-expect-error A pending turn may have no server-assigned ID.
        const turnId: string = payload.turn_id;
        void turnId;
    });
    return enabled;
}
