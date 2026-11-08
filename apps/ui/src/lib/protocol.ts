// What the core sends, as the protocol serializes it (`lucy_protocol::rpc`).

/** A core event, as the protocol serializes it (`lucy_protocol::rpc::Event`). */
export interface CoreEvent {
  type: string;
  turn_id?: number;
  text?: string;
  call?: { id: string; name: string; arguments: Record<string, unknown> };
  reason?: string;
  preview?: string;
  outcome?: string;
  error?: string;
  [key: string]: unknown;
}

/** The protocol this front end speaks (`lucy_protocol::rpc::PROTOCOL_VERSION`). */
export const PROTOCOL_VERSION = 5;
