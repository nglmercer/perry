/**
 * Extract text from a PDF buffer.
 */
export function parse(buf: Uint8Array): string {
  // Unreachable under perry — links to js_pdf_parse instead.
  throw new Error("Not implemented under V8 fallback");
}
