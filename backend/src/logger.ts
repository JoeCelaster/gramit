type Fields = Record<string, unknown>;

function emit(stream: NodeJS.WriteStream, level: string, message: string, fields?: Fields): void {
  const time = new Date().toISOString();
  const extra =
    fields && Object.keys(fields).length > 0
      ? ' ' +
        Object.entries(fields)
          .map(([k, v]) => `${k}=${typeof v === 'string' ? v : JSON.stringify(v)}`)
          .join(' ')
      : '';
  stream.write(`${time} ${level.padEnd(5)} ${message}${extra}\n`);
}

export const log = {
  info: (message: string, fields?: Fields) => emit(process.stdout, 'info', message, fields),
  warn: (message: string, fields?: Fields) => emit(process.stderr, 'warn', message, fields),
  error: (message: string, fields?: Fields) => emit(process.stderr, 'error', message, fields),
};
