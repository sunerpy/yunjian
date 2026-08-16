interface StructuredErrorReason {
  code?: unknown;
  message?: unknown;
  hint?: unknown;
}

function nonEmptyString(value: unknown): string | null {
  return typeof value === "string" && value.trim() !== "" ? value : null;
}

export function errorReason(cause: unknown, fallback: string): string {
  if (cause instanceof Error) {
    return nonEmptyString(cause.message) ?? fallback;
  }

  const text = nonEmptyString(cause);
  if (text !== null) {
    return text;
  }

  if (typeof cause === "object" && cause !== null) {
    const structured = cause as StructuredErrorReason;
    const code = nonEmptyString(structured.code);
    const message = nonEmptyString(structured.message);
    const hint = nonEmptyString(structured.hint);
    if (message !== null) {
      return `${code === null ? "" : `[${code}] `}${message}${hint === null ? "" : `；${hint}`}`;
    }
  }

  return fallback;
}
