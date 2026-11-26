import { expect, Page, test } from "@playwright/test";
import { mkdirSync } from "node:fs";

// The product as people use it: an empty workspace in which an administrator builds their own
// configuration from the components — here the expense-claim process of doc 14 (S3) — and
// then works with it. Every step saves a screenshot to e2e/screenshots/ (uploaded by CI).

mkdirSync("e2e/screenshots", { recursive: true });

async function shot(page: Page, name: string) {
  await page.waitForTimeout(150);
  await page.screenshot({ path: `e2e/screenshots/build-${name}.png`, fullPage: false });
}

function row(page: Page, text: string) {
  return page.getByTestId("grid-row").filter({ hasText: text });
}

async function openTable(page: Page, name: string) {
  await page.getByTestId(`nav-${name}`).click();
  await expect(page.getByTestId("table-title")).toHaveText(name);
}

/// Adds a field through the picker: type, name, then whatever the type needs.
async function addField(page: Page, type: string, name: string, fill?: (page: Page) => Promise<void>) {
  await page.getByTestId("add-field").click();
  await page.getByTestId(`type-${type}`).click();
  await page.getByTestId("field-name").fill(name);
  if (fill) await fill(page);
  await page.getByTestId("create-field").click();
  await expect(page.getByTestId("field-picker")).toBeHidden();
  await expect(page.locator("thead")).toContainText(name);
}

test.describe.serial("build your own: expense claims from scratch", () => {
  let page: Page;

  test.beforeAll(async ({ browser }) => {
    page = await browser.newPage();
    page.on("dialog", (dialog) => dialog.accept());
    await page.goto("/");
    await page.evaluate(() => window.localStorage.clear());
    await page.goto("/");
  });

  test.afterAll(async () => {
    await page.close();
  });

  test("a new workspace is empty, with a template gallery", async () => {
    await expect(page.getByTestId("empty-workspace")).toBeVisible();
    await expect(page.getByTestId("template-purchasing")).toContainText("Purchasing (sample)");
    await shot(page, "01-empty-workspace");
  });

  test("a base is created from scratch", async () => {
    await page.getByTestId("new-base-name").fill("Expenses");
    await page.getByTestId("new-base-description").fill("Expense claims, their lines and approval");
    await page.getByTestId("create-base").click();
    await expect(page.getByTestId("base-title")).toHaveText("◫ Expenses");
    await expect(page.getByTestId("nav-base-Expenses")).toBeVisible();
    await shot(page, "02-new-base");
  });

  test("a code list of expense categories", async () => {
    await page.getByTestId("nav-refdata").click();
    await page.getByTestId("new-set-name").fill("Expense categories");
    await page.getByTestId("new-set-code").fill("expense_category");
    await page.getByTestId("new-set-tree").check();
    await page.getByTestId("create-set").click();
    await expect(page.getByTestId("set-expense_category")).toBeVisible();
    for (const [name, code, parent] of [
      ["Travel", "TRAVEL", ""],
      ["Flights", "FLIGHT", "Travel"],
      ["Hotels", "HOTEL", "Travel"],
      ["Meals", "MEALS", ""],
    ]) {
      await page.getByTestId("item-name").fill(name);
      await page.getByTestId("item-code").fill(code);
      if (parent) await page.getByTestId("item-parent").selectOption({ label: parent });
      else await page.getByTestId("item-parent").selectOption("");
      await page.getByTestId("add-item").click();
      await expect(page.getByTestId("refdata-items")).toContainText(name);
    }
    await expect(page.getByTestId("refdata-items")).toContainText("Travel › Flights");
    await shot(page, "03-code-list");
  });

  test("a setting for the manager's approval limit", async () => {
    await page.getByTestId("nav-settings").click();
    await page.getByTestId("setting-key").fill("expense.manager_limit");
    await page.getByTestId("setting-type").selectOption("money");
    await page.getByTestId("setting-value").fill("1000 USD");
    await page.getByTestId("save-setting").click();
    await expect(page.getByTestId("settings")).toContainText("1,000.00 USD");
    await shot(page, "04-setting");
  });

  test("the claims table with a number and the claimant", async () => {
    await page.getByTestId("nav-base-link-Expenses").click();
    await page.getByTestId("new-table-name").fill("Expense claims");
    await page.getByTestId("new-table-primary").fill("Purpose");
    await page.getByTestId("create-table").click();
    await expect(page.getByTestId("table-title")).toHaveText("Expense claims");
    await addField(page, "auto_number", "Claim no", async (p) => {
      await p.getByTestId("field-pattern").fill("EXP-{YYYY}-{seq:4}");
    });
    await addField(page, "person", "Claimant", async (p) => {
      await p.getByTestId("field-role").selectOption("requester");
    });
    await shot(page, "05-claims-table");
  });

  test("the lines table: date, category, amount, and a two-way link to the claim", async () => {
    await page.getByTestId("nav-base-link-Expenses").click();
    await page.getByTestId("new-table-name").fill("Expense lines");
    await page.getByTestId("new-table-primary").fill("Description");
    await page.getByTestId("create-table").click();
    await expect(page.getByTestId("table-title")).toHaveText("Expense lines");
    await addField(page, "date", "Date");
    await addField(page, "code_list", "Category", async (p) => {
      await p.getByTestId("field-refset").selectOption({ label: "Expense categories" });
    });
    await addField(page, "currency", "Amount", async (p) => {
      await p.getByTestId("field-currency").selectOption("USD");
      await p.getByTestId("field-required").check();
    });
    await page.getByTestId("add-field").click();
    await page.getByTestId("type-search").fill("link");
    await page.getByTestId("type-link").click();
    await page.getByTestId("field-name").fill("Claim");
    await page.getByTestId("field-target").selectOption({ label: "Expense claims" });
    await page.getByTestId("field-single").check();
    await page.getByTestId("field-inverse").fill("Lines");
    await shot(page, "06-link-field");
    await page.getByTestId("create-field").click();
    await expect(page.getByTestId("field-picker")).toBeHidden();
    await expect(page.locator("thead")).toContainText("Claim");
  });

  test("the total rolls up the lines and a formula reads the setting", async () => {
    await openTable(page, "Expense claims");
    await expect(page.locator("thead")).toContainText("Lines");
    await addField(page, "rollup", "Total", async (p) => {
      await p.getByTestId("formula").fill("SUM({Lines}.{Amount})");
      await expect(p.getByTestId("formula-ok")).toContainText("money");
    });
    await page.getByTestId("add-field").click();
    await page.getByTestId("type-formula").click();
    await page.getByTestId("field-name").fill("Needs finance");
    await page.getByTestId("formula").fill('{Total} > PARAM("expense.manager_limit"');
    await expect(page.getByTestId("formula-error")).toBeVisible();
    await page.getByTestId("formula").fill('{Total} > PARAM("expense.manager_limit")');
    await expect(page.getByTestId("formula-ok")).toContainText("bool");
    await shot(page, "07-formula-with-setting");
    await page.getByTestId("create-field").click();
    await expect(page.locator("thead")).toContainText("Needs finance");
  });

  test("people are added and grouped", async () => {
    await page.getByTestId("nav-people").click();
    for (const [name, title, kind] of [
      ["Mia Miller", "Sales — submits expenses", "member"],
      ["Max Manager", "Head of sales — approves", "member"],
      ["Fiona Finance", "Finance — reviews and pays", "member"],
      ["Finance team", "", "group"],
    ]) {
      await page.getByTestId("person-name").fill(name);
      await page.getByTestId("person-title").fill(title);
      await page.getByTestId("person-kind").selectOption(kind);
      await page.getByTestId("add-person").click();
      await expect(page.getByTestId("people-page")).toContainText(name);
    }
    await page.getByTestId("member-Finance team-Fiona Finance").click();
    await expect(page.getByTestId("member-Finance team-Fiona Finance")).toHaveClass(/on/);
    await shot(page, "08-people");
  });

  test("a claim with three lines is entered and totalled", async () => {
    await openTable(page, "Expense claims");
    await page.getByTestId("new-record").click();
    await page.getByTestId("edit-Purpose").fill("Client visit Berlin");
    await page.getByTestId("save").click();
    await expect(page.getByTestId("panel-message")).toHaveText("Saved.");
    await expect(page.getByTestId("value-Claim no")).toContainText("EXP-");
    await page.getByTestId("close-panel").click();
    await openTable(page, "Expense lines");
    for (const [description, amount, category, date] of [
      ["Flight to Berlin", "420.00", "Travel", "2026-11-10"],
      ["Hotel, two nights", "380", "Travel", "2026-11-11"],
      ["Client dinner", "95.50", "Meals", "2026-11-11"],
    ]) {
      await page.getByTestId("new-record").click();
      await page.getByTestId("edit-Description").fill(description);
      await page.getByTestId("edit-Date").fill(date);
      await page.getByTestId("edit-Category").selectOption({ label: category });
      await page.getByTestId("edit-Amount").fill(amount);
      await page.getByTestId("edit-Claim").selectOption({ label: "Client visit Berlin" });
      await page.getByTestId("save").click();
      await expect(page.getByTestId("panel-message")).toHaveText("Saved.");
      await page.getByTestId("close-panel").click();
    }
    await expect(page.getByTestId("row-count")).toHaveText("3");
    await expect(page.locator("tfoot")).toContainText("895.50 USD");
    await openTable(page, "Expense claims");
    await expect(row(page, "Client visit Berlin")).toContainText("895.50 USD");
    await expect(row(page, "Client visit Berlin")).toContainText("Alice Admin");
    await expect(row(page, "Client visit Berlin")).toContainText("Flight to Berlin");
    await shot(page, "09-claim-totalled");
  });

  test("a required amount is required", async () => {
    await openTable(page, "Expense lines");
    await page.getByTestId("new-record").click();
    await page.getByTestId("edit-Description").fill("Taxi");
    await page.getByTestId("save").click();
    await expect(page.getByTestId("field-error")).toContainText("required");
    await page.getByTestId("close-panel").click();
  });

  test("changing the setting recomputes every formula that reads it", async () => {
    await page.getByTestId("nav-settings").click();
    await page.getByTestId("setting-key").fill("expense.manager_limit");
    await page.getByTestId("setting-type").selectOption("money");
    await page.getByTestId("setting-value").fill("500 USD");
    await page.getByTestId("save-setting").click();
    await expect(page.getByTestId("settings")).toContainText("500.00 USD");
    await openTable(page, "Expense claims");
    await expect(row(page, "Client visit Berlin")).toContainText("✓");
    await shot(page, "10-recomputed");
  });

  test("fields are renamed, and what others use cannot be deleted", async () => {
    await page.getByTestId("head-Purpose").click();
    await page.getByTestId("edit-field-name").fill("Trip purpose");
    await page.getByTestId("edit-field-help").fill("Why the money was spent");
    await page.getByTestId("save-field").click();
    await expect(page.locator("thead")).toContainText("Trip purpose");
    await page.getByTestId("head-Total").click();
    await page.getByTestId("delete-field").click();
    await expect(page.getByTestId("field-editor-message")).toContainText("used by the formula “Needs finance”");
    await shot(page, "11-field-in-use");
    await page.getByTestId("field-editor").getByRole("button", { name: "Cancel" }).click();
    await expect(page.locator("thead")).toContainText("Total");
  });

  test("the process is designed: states, buttons and an approval", async () => {
    await openTable(page, "Expense claims");
    await page.getByTestId("design-process").click();
    await expect(page.getByTestId("designer")).toBeVisible();
    // The starting point: Draft → Done. Done becomes Submitted, waiting for approval.
    await page.getByTestId("state-name-1").fill("Submitted");
    await page.getByTestId("state-category-1").selectOption("pending_approval");
    await page.getByTestId("state-locked-1").check();
    await page.getByTestId("state-required-1-Lines").click();
    for (const [name, category, locked] of [
      ["Approved", "in_progress", true],
      ["Paid", "done_success", false],
      ["Rejected", "done_failure", false],
    ] as const) {
      await page.getByTestId("add-state").click();
      const i = (await page.getByTestId("state-row").count()) - 1;
      await page.getByTestId(`state-name-${i}`).fill(name);
      await page.getByTestId(`state-category-${i}`).selectOption(category);
      if (locked) await page.getByTestId(`state-locked-${i}`).check();
    }
    await page.getByTestId("button-name-0").fill("Submit");
    for (const [name, from, to, capability] of [
      ["Pay", "Approved", "Paid", "expense.pay"],
      ["Revise", "Rejected", "Draft", ""],
      ["Withdraw", "Submitted", "Draft", ""],
    ]) {
      await page.getByTestId("add-button").click();
      const i = (await page.getByTestId("button-row").count()) - 1;
      await page.getByTestId(`button-name-${i}`).fill(name);
      await page.getByTestId(`button-from-${i}-${from}`).click();
      await page.getByTestId(`button-to-${i}`).selectOption({ label: to });
      if (capability) await page.getByTestId(`button-capability-${i}`).fill(capability);
    }
    // The approval: the manager, then finance only above the limit; finance is told to pay.
    await page.getByTestId("approval-enabled").check();
    await page.getByTestId("approval-name").fill("Expense approval");
    await page.getByTestId("approval-start").selectOption({ label: "Submit" });
    await page.getByTestId("approval-approved").selectOption({ label: "Approved" });
    await page.getByTestId("approval-rejected").selectOption({ label: "Rejected" });
    await page.getByTestId("approval-returned").selectOption({ label: "Draft" });
    await page.getByTestId("add-step").click();
    await page.getByTestId("step-name-0").fill("Manager");
    await page.getByTestId("step-0-add-approver").click();
    await page.getByTestId("step-0-approver-0-person").selectOption({ label: "Max Manager" });
    await page.getByTestId("add-step").click();
    await page.getByTestId("step-name-1").fill("Finance review");
    await page.getByTestId("step-1-add-approver").click();
    await page.getByTestId("step-1-approver-0-kind").selectOption("capability");
    await page.getByTestId("step-1-approver-0-capability").fill("expense.finance_review");
    await page.getByTestId("step-when-1-add").click();
    await page.getByTestId("step-when-1-field").selectOption({ label: "Needs finance" });
    await page.getByTestId("step-when-1-value").selectOption("yes");
    await page.getByTestId("add-step").click();
    await page.getByTestId("step-kind-2").selectOption("notify");
    await page.getByTestId("step-name-2").fill("Ready to pay");
    await page.getByTestId("step-2-add-approver").click();
    await page.getByTestId("step-2-approver-0-kind").selectOption("capability");
    await page.getByTestId("step-2-approver-0-capability").fill("expense.pay");
    // A state nothing leads to is refused, with the reason.
    await page.getByTestId("add-state").click();
    const extra = (await page.getByTestId("state-row").count()) - 1;
    await page.getByTestId(`state-name-${extra}`).fill("On hold");
    await page.getByTestId("publish-process").click();
    await expect(page.getByTestId("designer-message")).toContainText("the state “On hold” can never be reached");
    await shot(page, "14-design-refused");
    await page.getByTestId(`remove-state-${extra}`).click();
    await page.getByTestId("publish-process").click();
    await expect(page.getByTestId("designer-message")).toContainText("Published");
    await expect(page.getByTestId("designer")).toContainText("Approval “Expense approval” version 1");
    await shot(page, "15-process-designed");
  });

  test("access is given: employees, the manager, finance with a limit", async () => {
    await page.getByTestId("nav-access").click();
    await page.getByTestId("set-name").fill("Employee");
    for (const c of ["record.read", "record.create", "record.update"]) await page.getByTestId(`set-cap-${c}`).click();
    await page.getByTestId("create-cap-set").click();
    await expect(page.getByTestId("access-message")).toHaveText("Set created.");
    const give = async (to: string, what: string, place: string, scope = "all", reason = "") => {
      await page.getByTestId("grant-to").selectOption({ label: to });
      if (what.startsWith("set:")) {
        await page.getByTestId("grant-kind").selectOption("set");
        await page.getByTestId("grant-set").selectOption({ label: what.slice(4) });
      } else {
        await page.getByTestId("grant-kind").selectOption("capability");
        await page.getByTestId("grant-capability").fill(what);
      }
      await page.getByTestId("grant-place").selectOption({ label: place });
      await page.getByTestId("grant-scope").selectOption(scope);
      if (what === "expense.pay") {
        await page.getByTestId("grant-limit-add").click();
        await page.getByTestId("grant-limit-field").selectOption({ label: "Total" });
        await page.getByTestId("grant-limit-op").selectOption("le");
        await page.getByTestId("grant-limit-value").fill("10000 USD");
      }
      await page.getByTestId("grant-reason").fill(reason);
      await page.getByTestId("give-grant").click();
      await expect(page.getByTestId("access-message")).toHaveText("Given.");
    };
    await give("Mia Miller", "set:Employee", "the base Expenses", "own", "Submits her expenses");
    await give("Max Manager", "record.read", "the base Expenses", "all", "Approves the team's expenses");
    await give("Finance team (group)", "record.read", "the base Expenses");
    await give("Finance team (group)", "expense.finance_review", "  the table Expense claims");
    await give("Finance team (group)", "expense.pay", "  the table Expense claims", "all", "Pays approved claims");
    await expect(page.getByTestId("grants")).toContainText("“Total” ≤ 10,000.00 USD");
    await page.getByTestId("grants").scrollIntoViewIfNeeded();
    await shot(page, "16-access-given");
  });

  test("Mia enters and submits a claim; she sees only her own", async () => {
    await page.getByTestId("user-select").selectOption({ label: "Mia Miller — Sales — submits expenses" });
    await openTable(page, "Expense claims");
    await expect(page.getByTestId("row-count")).toHaveText("0");
    await page.getByTestId("new-record").click();
    await page.getByTestId("edit-Trip purpose").fill("Trade fair Paris");
    await page.getByTestId("save").click();
    await expect(page.getByTestId("panel-message")).toHaveText("Saved.");
    await page.getByTestId("close-panel").click();
    await openTable(page, "Expense lines");
    for (const [description, amount] of [["Stand rental", "600"], ["Hotel Paris", "650"]]) {
      await page.getByTestId("new-record").click();
      await page.getByTestId("edit-Description").fill(description);
      await page.getByTestId("edit-Amount").fill(amount);
      await page.getByTestId("edit-Claim").selectOption({ label: "Trade fair Paris" });
      await page.getByTestId("save").click();
      await expect(page.getByTestId("panel-message")).toHaveText("Saved.");
      await page.getByTestId("close-panel").click();
    }
    await expect(page.getByTestId("row-count")).toHaveText("2");
    await openTable(page, "Expense claims");
    await row(page, "Trade fair Paris").click();
    await expect(page.getByTestId("state")).toHaveText("Draft");
    await expect(page.getByTestId("would-ask")).toContainText("Max Manager");
    await expect(page.getByTestId("would-ask")).toContainText("Fiona Finance");
    await page.getByTestId("transition-submit").click();
    await expect(page.getByTestId("state")).toHaveText("Submitted");
    await expect(page.getByTestId("timeline")).toContainText("Max Manager");
    await shot(page, "17-mia-submitted");
    await page.getByTestId("close-panel").click();
  });

  test("Max approves; finance reviews because the total is above the limit", async () => {
    await page.getByTestId("user-select").selectOption({ label: "Max Manager — Head of sales — approves" });
    await page.getByTestId("nav-approvals").click();
    await page.getByTestId("task-card").filter({ hasText: "Trade fair Paris" }).click();
    await page.getByTestId("task-comment").fill("Good value");
    await page.getByTestId("task-approve").click();
    await expect(page.getByTestId("task-done")).toContainText("your decision is recorded");
    await page.getByTestId("user-select").selectOption({ label: "Fiona Finance — Finance — reviews and pays" });
    await page.getByTestId("nav-approvals").click();
    await page.getByTestId("task-card").filter({ hasText: "Trade fair Paris" }).click();
    await expect(page.getByTestId("timeline")).toContainText("Good value");
    await shot(page, "18-finance-review");
    await page.getByTestId("task-approve").click();
    await expect(page.getByTestId("task-done")).toContainText("Approved");
  });

  test("finance pays; Mia is told", async () => {
    await page.getByTestId("nav-inbox").click();
    await expect(page.getByTestId("inbox")).toContainText("“Trade fair Paris”: Ready to pay");
    await openTable(page, "Expense claims");
    await row(page, "Trade fair Paris").click();
    await page.getByTestId("transition-pay").click();
    await expect(page.getByTestId("state")).toHaveText("Paid");
    await shot(page, "19-paid");
    await page.getByTestId("close-panel").click();
    await page.getByTestId("user-select").selectOption({ label: "Mia Miller — Sales — submits expenses" });
    await page.getByTestId("nav-inbox").click();
    await expect(page.getByTestId("inbox")).toContainText("“Trade fair Paris” was approved");
    await shot(page, "20-mia-told");
    await page.getByTestId("user-select").selectOption({ label: "Alice Admin — Workspace administrator" });
  });

  test("the process grows: a timeout with escalation and a fill-in step (version 2)", async () => {
    await openTable(page, "Expense claims");
    await addField(page, "date", "Paid on");
    await page.getByTestId("design-process").click();
    await expect(page.getByTestId("step-name-0")).toHaveValue("Manager");
    await page.getByTestId("step-timeout-0").fill("48");
    await page.getByTestId("step-0-add-escalate").click();
    await page.getByTestId("step-0-escalate-person").selectOption({ label: "Alice Admin" });
    await page.getByTestId("add-step").click();
    await page.getByTestId("step-kind-3").selectOption("fill_in");
    await page.getByTestId("step-name-3").fill("Payment date");
    await page.getByTestId("step-3-add-approver").click();
    await page.getByTestId("step-3-approver-0-person").selectOption({ label: "Fiona Finance" });
    await page.getByTestId("step-3-field-Paid on").click();
    // Branches that run at the same time are designed the same way.
    await page.getByTestId("add-step").click();
    await page.getByTestId("step-kind-4").selectOption("parallel");
    await expect(page.getByTestId("branch-4-1")).toBeVisible();
    await page.getByTestId("add-step-4-b0").click();
    await page.getByTestId("step-name-4-b0-0").fill("Legal");
    await page.getByTestId("step-row").last().scrollIntoViewIfNeeded();
    await shot(page, "23-designer-branches");
    await page.getByTestId("remove-step-4").click();
    await page.getByTestId("publish-process").click();
    await expect(page.getByTestId("designer-message")).toContainText("Published");
    await expect(page.getByTestId("designer")).toContainText("version 2");
    await shot(page, "24-process-version-2");
  });

  test("a second claim: the manager approves, finance fills in the payment date", async () => {
    await page.getByTestId("user-select").selectOption({ label: "Mia Miller — Sales — submits expenses" });
    await openTable(page, "Expense claims");
    await page.getByTestId("new-record").click();
    await page.getByTestId("edit-Trip purpose").fill("Client lunch");
    await page.getByTestId("save").click();
    await expect(page.getByTestId("panel-message")).toHaveText("Saved.");
    await page.getByTestId("close-panel").click();
    await openTable(page, "Expense lines");
    await page.getByTestId("new-record").click();
    await page.getByTestId("edit-Description").fill("Lunch with Contoso");
    await page.getByTestId("edit-Amount").fill("80");
    await page.getByTestId("edit-Claim").selectOption({ label: "Client lunch" });
    await page.getByTestId("save").click();
    await expect(page.getByTestId("panel-message")).toHaveText("Saved.");
    await page.getByTestId("close-panel").click();
    await openTable(page, "Expense claims");
    await row(page, "Client lunch").click();
    await page.getByTestId("transition-submit").click();
    await expect(page.getByTestId("state")).toHaveText("Submitted");
    await expect(page.getByTestId("timeline")).toContainText("due");
    await page.getByTestId("close-panel").click();
    await page.getByTestId("user-select").selectOption({ label: "Max Manager — Head of sales — approves" });
    await page.getByTestId("nav-approvals").click();
    await page.getByTestId("task-card").filter({ hasText: "Client lunch" }).click();
    await page.getByTestId("task-approve").click();
    await expect(page.getByTestId("task-done")).toContainText("your decision is recorded");
    await page.getByTestId("user-select").selectOption({ label: "Fiona Finance — Finance — reviews and pays" });
    await page.getByTestId("nav-approvals").click();
    await page.getByTestId("task-card").filter({ hasText: "Client lunch" }).click();
    await page.getByTestId("task-submit").click();
    await expect(page.getByTestId("task-message")).toContainText("fill in Paid on first");
    await page.getByTestId("fill-Paid on").fill("2026-12-01");
    await shot(page, "25-fill-in");
    await page.getByTestId("task-submit").click();
    await expect(page.getByTestId("task-done")).toContainText("Approved");
    await page.getByTestId("user-select").selectOption({ label: "Alice Admin — Workspace administrator" });
  });

  test("a template installs next to what was built", async () => {
    await page.getByTestId("nav-home").click();
    await page.getByTestId("install-purchasing").click();
    await expect(page.getByTestId("template-purchasing")).toContainText("installed");
    await expect(page.getByTestId("base-cards")).toContainText("Purchasing");
    await expect(page.getByTestId("base-cards")).toContainText("Expenses");
    await expect(page.getByTestId("nav-Purchase requests")).toBeVisible();
    await shot(page, "26-template-installed");
  });

  test("everything built is in the audit chain", async () => {
    await page.getByTestId("nav-audit").click();
    for (const action of ["base.create", "refset.create", "table.create", "field.create", "setting.update", "member.create", "process.publish", "access.grant", "approval.approve", "template.install"]) {
      await expect(page.getByTestId("audit-entries")).toContainText(action);
    }
    await page.getByTestId("verify").click();
    await expect(page.getByTestId("verdict")).toContainText("Chain intact");
    await shot(page, "27-audit");
  });
});
