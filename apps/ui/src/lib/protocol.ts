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

/** A picture kept with a chat (`lucy_protocol::rpc::ImageRef`); `images.get` fetches it. */
export interface ImageRef {
  id: string;
  media_type: string;
  /** She made it, rather than being shown it. */
  made?: boolean;
}

/** A picture to send (`lucy_protocol::rpc::ImageData`): its type and base64 bytes. */
export interface ImageData {
  media_type: string;
  data: string;
}

/** The protocol this front end speaks (`lucy_protocol::rpc::PROTOCOL_VERSION`). */
export const PROTOCOL_VERSION = 5;
