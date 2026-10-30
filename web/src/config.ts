import { z } from "zod";

export const LOG_LEVELS = ["debug", "info", "warn", "error", "silent"] as const;

export type LogLevel = (typeof LOG_LEVELS)[number];

const zLogLevel = z.enum(LOG_LEVELS);

export interface Config {
  logLevel: LogLevel;
}

export function readConfig(env: ImportMetaEnv): Config {
  const fallback: LogLevel = env.DEV ? "debug" : "warn";
  const parsed = zLogLevel.safeParse(env.VITE_LOG_LEVEL);
  return { logLevel: parsed.success ? parsed.data : fallback };
}

export const config: Config = readConfig(import.meta.env);
