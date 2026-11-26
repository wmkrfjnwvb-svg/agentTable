import { expect, Page, test } from "@playwright/test";
import { mkdirSync } from "node:fs";

// One walk through the product as it stands, in order, as different people. Each step saves
// a screenshot to e2e/screenshots/ (uploaded by CI).

const people = {
  alice: "prn_00000000000000000000000001",
  bob: "prn_00000000000000000000000002",
  carol: "prn_00000000000000000000000003",
  dave: "prn_00000000000000000000000004",
  gina: "prn_00000000000000000000000005",
  assistant: "prn_00000000000000000000000006",
  erin: "prn_00000000000000000000000007",
};

mkdirSync("e2e/screenshots", { recursive: true });

async function shot(page: Page, name: string) {
  await page.waitForTimeout(150);
  await page.screenshot({ path: `e2e/screenshots/${name}.png`, fullPage: false });
}

async function signIn(page: Page, who: keyof typeof people) {
  await page.getByTestId("user-select").selectOption(people[who]);
}

async function openTable(page: Page, name: string) {
  await page.getByTestId(`nav-${name}`).click();
  await expect(page.getByTestId("table-title")).toHaveText(name);
}

function row(page: Page, text: string) {
  return page.getByTestId("grid-row").filter({ hasText: text });
}

test.describe.serial("agenttable end to end", () => {
  let page: Page;

  test.beforeAll(async ({ browser }) => {
    page = await browser.newPage();
    await page.goto("/");
    await page.evaluate(() => window.localStorage.clear());
    await page.goto("/");
  });

  test.afterAll(async () => {
    await page.close();
  });

  test("the overview shows the tables and what is built", async () => {
    await expect(page.getByTestId("base-cards")).toContainText("Purchasing");
    await page.getByTestId("nav-status").click();
    await expect(page.getByTestId("overview")).toBeVisible();
    await expect(page.getByTestId("components")).toContainText("mt-formula");
    await expect(page.getByTestId("components")).toContainText("placeholder");
    await shot(page, "01-overview");
  });

  test("purchase requests compute their totals", async () => {
    await openTable(page, "Purchase requests");
    await expect(page.getByTestId("row-count")).toHaveText("5");
    await expect(row(page, "Trade fair booth")).toContainText("27,648.00 USD");
    await expect(page.locator("tfoot")).toContainText("46,530.72 USD");
    await shot(page, "02-purchase-requests");
  });

  test("changing a line recomputes the request and the supplier", async () => {
    await openTable(page, "Request lines");
    await row(page, "Ergonomic chair").click();
    await expect(page.getByTestId("panel-title")).toHaveText("Ergonomic chair");
    await page.getByTestId("edit-Quantity").fill("8");
    await page.getByTestId("save").click();
    await expect(page.getByTestId("panel-message")).toHaveText("Saved.");
    await expect(page.getByTestId("value-Amount")).toContainText("2,560.00 USD");
    await shot(page, "03-line-changed");
    await page.getByTestId("close-panel").click();
    await openTable(page, "Purchase requests");
    // 2,560.00 + 8 % tax = 2,764.80
    await expect(row(page, "Office chairs")).toContainText("2,764.80 USD");
    await openTable(page, "Suppliers");
    await expect(row(page, "Contoso Supplies")).toContainText("2,764.80 USD");
    await shot(page, "04-recomputed-across-tables");
  });

  test("lines of a request under approval are locked", async () => {
    await openTable(page, "Request lines");
    await row(page, "Laptop 14").click();
    await page.getByTestId("edit-Quantity").fill("4");
    await page.getByTestId("save").click();
    await expect(page.getByTestId("panel-message")).toContainText("is Submitted: its lines cannot change");
    await shot(page, "05a-line-locked");
    await page.getByTestId("close-panel").click();
    await openTable(page, "Suppliers");
  });

  test("invalid values are refused with a reason", async () => {
    await row(page, "Northwind Traders").click();
    await page.getByTestId("edit-Contact email").fill("not-an-email");
    await page.getByTestId("save").click();
    await expect(page.getByTestId("field-error")).toBeVisible();
    await shot(page, "05-validation");
    await page.getByTestId("close-panel").click();
  });

  test("Bob sees only his own requests and no bank details", async () => {
    await signIn(page, "bob");
    await openTable(page, "Purchase requests");
    await expect(page.getByTestId("row-count")).toHaveText("3");
    await expect(page.getByTestId("grid")).not.toContainText("Trade fair booth");
    await shot(page, "06-bob-requests");
    await openTable(page, "Suppliers");
    await expect(page.getByTestId("hidden-fields")).toContainText("IBAN");
    await shot(page, "07-bob-suppliers");
  });

  test("a guest sees only her own company", async () => {
    await signIn(page, "gina");
    await openTable(page, "Suppliers");
    await expect(page.getByTestId("row-count")).toHaveText("1");
    await expect(page.getByTestId("grid")).toContainText("Northwind Traders");
    await shot(page, "08-guest");
    await page.getByTestId("nav-Purchase requests").click();
    await expect(page.getByTestId("no-access")).toBeVisible();
  });

  test("an AI agent acting for Bob sees what Bob sees", async () => {
    await signIn(page, "assistant");
    await page.getByTestId("behalf-select").selectOption(people.bob);
    await openTable(page, "Purchase requests");
    await expect(page.getByTestId("row-count")).toHaveText("3");
    await shot(page, "09-agent-for-bob");
  });

  test("a formula field is written, checked and added", async () => {
    await signIn(page, "alice");
    await openTable(page, "Purchase requests");
    await page.getByTestId("add-field").click();
    await page.getByTestId("type-search").fill("calculation");
    await page.getByTestId("type-formula").click();
    await page.getByTestId("field-name").fill("Tax %");
    await page.getByTestId("formula").fill("ROUND({Tax} / {Total} * 100, 1");
    await expect(page.getByTestId("formula-error")).toBeVisible();
    await shot(page, "10-formula-error");
    await page.getByTestId("formula").fill("ROUND({Tax} / {Total} * 100, 1)");
    await expect(page.getByTestId("formula-ok")).toContainText("number");
    await shot(page, "11-formula-checked");
    await page.getByTestId("create-field").click();
    await expect(page.getByTestId("field-picker")).toBeHidden();
    await expect(page.locator("thead")).toContainText("Tax %");
    await expect(row(page, "Office chairs")).toContainText("7.4");
    await shot(page, "12-formula-column");
  });

  test("records are grouped with subtotals and filtered", async () => {
    await page.getByTestId("group-by").selectOption({ label: "Status" });
    await expect(page.getByTestId("group-head").first()).toBeVisible();
    expect(await page.getByTestId("group-head").count()).toBeGreaterThanOrEqual(3);
    await shot(page, "13-grouped");
    await page.getByTestId("group-by").selectOption("");
    await page.getByTestId("add-filter").click();
    await page.getByTestId("filter-field").selectOption({ label: "Total" });
    await page.getByTestId("filter-op").selectOption("gt");
    await page.getByTestId("filter-value").fill("5000");
    await expect(page.getByTestId("row-count")).toHaveText("3");
    await shot(page, "14-filtered");
  });

  test("approval limits and separation of duties are explained", async () => {
    await signIn(page, "carol");
    await openTable(page, "Purchase requests");
    await row(page, "Trade fair booth").click();
    await page.getByTestId("approve").click();
    await expect(page.getByTestId("decision-Approve")).toContainText("not allowed");
    await expect(page.getByTestId("decision-Approve")).toContainText("condition");
    await page.getByTestId("decision-Approve").scrollIntoViewIfNeeded();
    await shot(page, "15-approve-over-limit");
    await row(page, "Design tool licences").click();
    await page.getByTestId("approve").click();
    await expect(page.getByTestId("decision-Approve")).toContainText("separation of duties");
    await row(page, "Laptops for new hires").click();
    await page.getByTestId("approve").click();
    await expect(page.getByTestId("decision-Approve")).toContainText("allowed");
    await page.getByTestId("decision-Approve").scrollIntoViewIfNeeded();
    await shot(page, "16-approve-allowed");
    await page.getByTestId("close-panel").click();
  });

  test("access can be explained for anyone", async () => {
    await page.getByTestId("nav-access").click();
    await page.getByTestId("explain-who").selectOption(people.carol);
    await page.getByTestId("explain-capability").selectOption("request.approve");
    await page.getByTestId("explain-table").selectOption({ label: "Purchase requests" });
    await page.getByTestId("explain-record").selectOption({ label: "Trade fair booth" });
    await page.getByTestId("explain-ask").click();
    await expect(page.getByTestId("explain-answer")).toContainText("Not allowed");
    await shot(page, "17-explain");
  });

  test("a draft is submitted and routed by the access rules", async () => {
    await signIn(page, "bob");
    await openTable(page, "Purchase requests");
    await row(page, "Office chairs").click();
    await expect(page.getByTestId("state")).toHaveText("Draft");
    await expect(page.getByTestId("would-ask")).toContainText("Carol Controller or Dave Finance");
    await shot(page, "21-draft-would-ask");
    await page.getByTestId("transition-submit").click();
    await expect(page.getByTestId("state")).toHaveText("Submitted");
    await expect(page.getByTestId("timeline")).toContainText("Carol Controller");
    await expect(page.getByTestId("timeline")).toContainText("Dave Finance");
    await expect(page.getByTestId("edit-Title")).toHaveCount(0);
    await shot(page, "22-submitted");
    await page.getByTestId("close-panel").click();
  });

  test("the approver decides from My approvals", async () => {
    await signIn(page, "carol");
    await page.getByTestId("nav-approvals").click();
    await expect(page.getByTestId("task-card")).toHaveCount(2);
    await page.getByTestId("task-card").filter({ hasText: "Office chairs" }).click();
    await expect(page.getByTestId("task-record")).toHaveText("Office chairs");
    await page.getByTestId("task-comment").fill("Fine, standard model");
    await shot(page, "23-my-tasks");
    await page.getByTestId("task-approve").click();
    await expect(page.getByTestId("task-done")).toContainText("Approved");
    await expect(page.getByTestId("task-card")).toHaveCount(1);
    await shot(page, "24-approved");
  });

  test("the requester is told in the inbox", async () => {
    await signIn(page, "bob");
    await page.getByTestId("nav-inbox").click();
    await expect(page.getByTestId("inbox")).toContainText("“Office chairs” was approved");
    await shot(page, "25-inbox");
  });

  test("a rejection needs a reason; above the limit the CFO decides", async () => {
    await signIn(page, "dave");
    await page.getByTestId("nav-approvals").click();
    await page.getByTestId("task-card").filter({ hasText: "Laptops for new hires" }).click();
    await page.getByTestId("task-reject").click();
    await expect(page.getByTestId("task-message")).toContainText("give a reason");
    await page.getByTestId("task-comment").fill("Buy refurbished");
    await page.getByTestId("task-reject").click();
    await expect(page.getByTestId("task-done")).toContainText("Rejected");
    await signIn(page, "erin");
    await page.getByTestId("nav-approvals").click();
    await page.getByTestId("task-card").filter({ hasText: "Trade fair booth" }).click();
    await expect(page.getByTestId("timeline")).toContainText("Dave Finance");
    await shot(page, "26-cfo-task");
    await page.getByTestId("task-approve").click();
    await expect(page.getByTestId("task-done")).toContainText("Approved");
    await expect(page.getByTestId("no-tasks")).toBeVisible();
  });

  test("finance places the order", async () => {
    await signIn(page, "dave");
    await openTable(page, "Purchase requests");
    await row(page, "Office chairs").click();
    await expect(page.getByTestId("state")).toHaveText("Approved");
    await page.getByTestId("transition-order").click();
    await expect(page.getByTestId("state")).toHaveText("Ordered");
    await expect(page.getByTestId("approval-status")).toHaveText("approved");
    await shot(page, "27-ordered");
    await page.getByTestId("close-panel").click();
  });

  test("the process design is shown", async () => {
    await page.getByTestId("nav-approvals").click();
    await page.getByTestId("tab-design").click();
    await expect(page.getByTestId("process-design")).toContainText("Erin CFO");
    await expect(page.getByTestId("process-design")).toContainText("whoever may “request.approve” it");
    await shot(page, "28-process-design");
    await page.getByTestId("tab-all").click();
    await expect(page.getByTestId("all-approvals")).toContainText("Trade fair booth");
    await shot(page, "29-all-approvals");
  });

  test("reference data shows the cost-centre tree", async () => {
    await page.getByTestId("nav-refdata").click();
    await page.getByTestId("set-cost_centre").click();
    await expect(page.getByTestId("refdata-items")).toContainText("Sales EMEA");
    await shot(page, "18-reference-data");
  });

  test("the audit chain verifies and detects tampering", async () => {
    await signIn(page, "carol");
    await page.getByTestId("nav-audit").click();
    await expect(page.getByTestId("no-access")).toBeVisible();
    await signIn(page, "dave");
    await expect(page.getByTestId("audit-entries")).toContainText("record.update");
    await expect(page.getByTestId("audit-entries")).toContainText("approval.approve");
    await page.getByTestId("verify").click();
    await expect(page.getByTestId("verdict")).toContainText("Chain intact");
    await page.getByTestId("tamper").click();
    await expect(page.getByTestId("tamper-result")).toContainText("Modified");
    await shot(page, "19-audit");
  });

  test("planned modules are shown as placeholders", async () => {
    await page.getByTestId("nav-soon-automations").click();
    await expect(page.getByTestId("placeholder")).toContainText("Planned for wave 5");
    await shot(page, "20-placeholder");
  });
});
