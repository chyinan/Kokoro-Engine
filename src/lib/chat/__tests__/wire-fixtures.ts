import type { StreamChatResponse } from "../../kokoro-bridge";

export function chatReply(
  reply: Pick<StreamChatResponse, "conversation_id"> & Partial<StreamChatResponse>,
): StreamChatResponse {
  return {
    text: "",
    status: "completed",
    client_request_id: null,
    user_message_id: null,
    assistant_message_id: null,
    ...reply,
  };
}
