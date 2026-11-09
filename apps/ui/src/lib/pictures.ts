// Pictures you give her: files pasted, dropped or chosen, read into what `chat.send` takes.

import type { ImageData } from "./protocol.ts";

/** The picture types she accepts (the same as the core's). */
export const PICTURE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/** The largest picture she takes (the core's limit). */
const MAX_BYTES = 20 * 1024 * 1024;

/** A picture waiting to be sent, with its name for showing. */
export interface Attached extends ImageData {
  name: string;
}

/** Read `file` as a picture, or say why it can't be one. */
export async function readPicture(file: File): Promise<Attached> {
  if (!PICTURE_TYPES.includes(file.type)) {
    throw new Error(`${file.name} isn't a picture she can see (PNG, JPEG, GIF or WebP)`);
  }
  if (file.size > MAX_BYTES) {
    throw new Error(`${file.name} is over ${MAX_BYTES / (1024 * 1024)} MB`);
  }
  const url = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error(`couldn't read ${file.name}`));
    reader.readAsDataURL(file);
  });
  return { name: file.name, media_type: file.type, data: url.slice(url.indexOf(",") + 1) };
}

/** A picture as a `data:` URL, for an `<img>`. */
export function pictureUrl(picture: ImageData): string {
  return `data:${picture.media_type};base64,${picture.data}`;
}
