import { defineConfig } from "vitest/config";

// Frontend test runner. Node environment is sufficient: the suites cover pure
// store logic and the IPC command contract, neither of which needs a DOM.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
