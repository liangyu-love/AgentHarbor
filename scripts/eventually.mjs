/**
 * Retry an assertion until it holds or `timeout` passes, then rethrow its
 * last failure. A UI check right after an action reads state that renders
 * a moment later on a slow machine (a CI runner); one already true passes
 * at once.
 */
export async function eventually(check, timeout = 5000) {
  const deadline = Date.now() + timeout;
  for (;;) {
    try {
      return await check();
    } catch (error) {
      if (Date.now() > deadline) throw error;
      await new Promise(resolve => setTimeout(resolve, 50));
    }
  }
}
