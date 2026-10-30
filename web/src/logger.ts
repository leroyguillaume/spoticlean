import { config, LOG_LEVELS, type LogLevel } from "./config";

export type LogFields = Record<string, unknown>;

export interface Logger {
  debug(event: string, fields?: LogFields): void;
  info(event: string, fields?: LogFields): void;
  warn(event: string, fields?: LogFields): void;
  error(event: string, fields?: LogFields): void;
}

type Level = Exclude<LogLevel, "silent">;

const sinks: Record<Level, (...data: unknown[]) => void> = {
  debug: console.debug,
  info: console.info,
  warn: console.warn,
  error: console.error,
};

export function createLogger(threshold: LogLevel): Logger {
  const enabled = (level: Level) => LOG_LEVELS.indexOf(level) >= LOG_LEVELS.indexOf(threshold);
  const log =
    (level: Level) =>
    (event: string, fields: LogFields = {}) => {
      if (enabled(level)) {
        sinks[level](event, fields);
      }
    };
  return { debug: log("debug"), info: log("info"), warn: log("warn"), error: log("error") };
}

export const logger: Logger = createLogger(config.logLevel);
