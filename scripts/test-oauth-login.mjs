import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

export async function testOAuthLogin({ page, reset, captureDir }) {
  const start = () => page.getByRole("button", { name: "OAuth 登录", exact: true });
  const cancel = () => page.getByRole("button", { name: "取消登录", exact: true });
  await reset("accounts");
  await start().click();
  await page.getByRole("status").filter({ hasText: "等待浏览器登录" }).waitFor();
  assert.equal(await start().count(), 0);
  assert.equal(await page.evaluate(() => window.review.loginCalls.length), 1);
  if (captureDir) {
    for (const [width, height] of [[1200, 760], [390, 844]]) {
      await page.setViewportSize({ width, height });
      await page.screenshot({ path: path.join(captureDir, `oauth-pending-${width}.png`), animations: "disabled" });
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    }
    await page.setViewportSize({ width: 1200, height: 700 });
  }
  await page.evaluate(() => { window.review.cancelHold = true; });
  await cancel().click();
  const cancelling = page.getByRole("button", { name: "正在取消登录…", exact: true });
  await cancelling.waitFor();
  assert.equal(await cancelling.isDisabled(), true);
  await page.evaluate(() => window.review.releaseCancel());
  await start().waitFor();
  await start().click();
  await cancel().waitFor();
  assert.equal(await page.evaluate(() => window.review.loginCalls.length), 2);
  assert.equal(await page.evaluate(() => window.review.loginCalls[0].attemptId !== window.review.loginCalls[1].attemptId), true);
  await page.evaluate(() => window.review.loginCalls[0].resolve());
  await page.waitForTimeout(80);
  assert.equal(await cancel().isVisible(), true);
  await page.evaluate(() => window.review.loginCalls[1].resolve());
  await start().waitFor();
  console.log("PASS OAuth cancel waits for cleanup, restarts without relaunch, and ignores stale success");

  await start().click();
  await cancel().waitFor();
  await page.evaluate(() => { window.review.cancelHold = false; window.review.cancelFailures = 1; });
  await cancel().click();
  await page.waitForFunction(() => window.review.cancelCalls.length === 2);
  await page.waitForTimeout(80);
  await eventually(async () => assert.equal(await cancel().isEnabled(), true));
  await cancel().click();
  await start().waitFor();
  await start().click();
  await cancel().waitFor();
  await page.evaluate(() => window.review.loginCalls[2].reject("accounts.loginFailed"));
  await page.waitForTimeout(80);
  assert.equal(await cancel().isVisible(), true);
  await page.evaluate(() => window.review.loginCalls[3].reject("accounts.loginTimeout"));
  await start().waitFor();
  console.log("PASS OAuth cancellation failure can retry, stale errors are ignored, and timeout permits new login");

  await start().click();
  await cancel().waitFor();
  await page.evaluate(() => { window.review.cancelResult = false; });
  await cancel().click();
  await page.waitForFunction(() => window.review.cancelCalls.length === 4);
  await page.waitForTimeout(80);
  await eventually(async () => assert.equal(await cancel().isVisible(), true));
  await page.evaluate(() => window.review.loginCalls[4].resolve());
  await start().waitFor();
  console.log("PASS OAuth completion wins a late cancellation without dropping the account result");
  await start().click();
  await cancel().waitFor();
  await page.evaluate(() => { window.review.oldLogin = window.review.loginCalls[5]; window.review.mount("overview"); });
  await page.waitForFunction(() => window.review.cancelCalls.length === 1);
  await page.evaluate(() => window.review.oldLogin.resolve());
  await page.evaluate(() => window.review.mount("accounts"));
  await start().waitFor();
  await start().click();
  await cancel().waitFor();
  await page.evaluate(() => window.review.loginCalls[0].reject("accounts.loginFailed"));
  await start().waitFor();
  console.log("PASS OAuth unmount cancels its own attempt and failure allows retry");

  await reset("overview");
  await page.evaluate(() => { window.review.emptyAccounts = true; window.review.mount("accounts"); });
  const emptyAdd = page.getByRole("button", { name: "添加账号", exact: true });
  await emptyAdd.waitFor();
  await emptyAdd.click();
  await cancel().waitFor();
  assert.equal(await emptyAdd.isDisabled(), true);
  await cancel().click();
  await start().waitFor();
  assert.equal(await emptyAdd.isEnabled(), true);
  console.log("PASS OAuth empty account page uses the same cancellable login flow");

}
