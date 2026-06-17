export interface ModActionEnvelope {
  action: string;
  data?: unknown;
}

export type ModActionHandler = (action: ModActionEnvelope) => void | Promise<void>;
export type ModActionHandlerMap = Record<string, ModActionHandler>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function getModActionFromEvent(event: Event): ModActionEnvelope | null {
  if (!(event instanceof CustomEvent)) return null;
  const detail = event.detail;
  if (!isRecord(detail)) return null;
  if (typeof detail.action !== "string" || detail.action.trim() === "") return null;
  return {
    action: detail.action,
    data: "data" in detail ? detail.data : undefined,
  };
}

export function createModActionDispatcher(handlers: ModActionHandlerMap) {
  return async (action: ModActionEnvelope): Promise<boolean> => {
    const handler = handlers[action.action];
    if (!handler) return false;
    await handler(action);
    return true;
  };
}
