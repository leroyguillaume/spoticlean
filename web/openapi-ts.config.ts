import { defineConfig } from "@hey-api/openapi-ts";

type Schema = { [key: string]: unknown };

const isSchema = (value: unknown): value is Schema => typeof value === "object" && value !== null;

// The zod plugin turns every (u)int64 into a `bigint`, which breaks arithmetic
// and `JSON` round-trips. The API's 64-bit integers are durations in
// milliseconds, far below 2^53, so they are read as plain numbers.
const dropInt64Formats = (node: unknown): void => {
  if (Array.isArray(node)) {
    node.forEach(dropInt64Formats);
  } else if (isSchema(node)) {
    if (node.format === "int64" || node.format === "uint64") {
      delete node.format;
    }
    Object.values(node).forEach(dropInt64Formats);
  }
};

export default defineConfig({
  input: "openapi.json",
  output: "src/api/generated",
  parser: {
    patch: {
      input: dropInt64Formats,
    },
  },
  plugins: ["zod"],
});
