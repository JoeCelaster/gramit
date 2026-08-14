/**
 * Every failure the backend can report is one of these codes. The daemon keys its
 * desktop notifications off `code`, never off the human-readable message, so codes
 * are part of the wire contract — rename them only alongside the daemon.
 */
export type ErrorCode =
  | 'INVALID_REQUEST'
  | 'EMPTY_TEXT'
  | 'TOO_LONG'
  | 'NO_API_KEY'
  | 'UPSTREAM_TIMEOUT'
  | 'RATE_LIMITED'
  | 'UPSTREAM_ERROR'
  | 'BAD_UPSTREAM_RESPONSE'
  | 'NOT_FOUND'
  | 'INTERNAL';

export class AppError extends Error {
  readonly code: ErrorCode;
  readonly status: number;
  /** Whether the daemon should suggest "try again" in its toast. */
  readonly retryable: boolean;

  constructor(code: ErrorCode, status: number, message: string, retryable = false) {
    super(message);
    this.name = 'AppError';
    this.code = code;
    this.status = status;
    this.retryable = retryable;
  }

  static invalidRequest(message: string): AppError {
    return new AppError('INVALID_REQUEST', 400, message);
  }

  static emptyText(): AppError {
    return new AppError('EMPTY_TEXT', 400, 'No text to correct.');
  }

  static tooLong(length: number, max: number): AppError {
    return new AppError('TOO_LONG', 413, `Text is ${length} characters; the limit is ${max}.`);
  }

  static noApiKey(message = 'Azure OpenAI is not configured on the backend.'): AppError {
    return new AppError('NO_API_KEY', 503, message);
  }

  static upstreamTimeout(ms: number): AppError {
    return new AppError('UPSTREAM_TIMEOUT', 504, `Azure OpenAI did not respond within ${ms}ms.`, true);
  }

  static rateLimited(): AppError {
    return new AppError('RATE_LIMITED', 429, 'Azure OpenAI rate limit reached.', true);
  }

  static upstream(message: string): AppError {
    return new AppError('UPSTREAM_ERROR', 502, message, true);
  }

  static badUpstreamResponse(message: string): AppError {
    return new AppError('BAD_UPSTREAM_RESPONSE', 502, message, true);
  }
}

export interface ErrorBody {
  error: {
    code: ErrorCode;
    message: string;
    retryable: boolean;
  };
}

export function toErrorBody(err: AppError): ErrorBody {
  return { error: { code: err.code, message: err.message, retryable: err.retryable } };
}
