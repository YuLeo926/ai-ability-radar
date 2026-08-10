export function createFakeProviderHarness({ result = { output: "done" }, error = null } = {}) {
  const events = [];
  return {
    events,
    loadProvider: async (id, options) => {
      events.push({ type: "load", id, options });
      return {
        callApi: async (prompt, context) => {
          events.push({
            type: "call",
            prompt,
            context,
            privateEnvironmentVisible: process.env.PRIVATE_PROVIDER_SECRET !== undefined,
            nodeOptionsVisible: process.env.NODE_OPTIONS !== undefined,
          });
          if (error) {
            throw error;
          }
          return result;
        },
      };
    },
  };
}
