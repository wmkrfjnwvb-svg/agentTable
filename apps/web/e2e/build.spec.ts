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

  test("a template installs next to what was built", async () => {
    await page.getByTestId("nav-home").click();
    await page.getByTestId("install-purchasing").click();
    await expect(page.getByTestId("template-purchasing")).toContainText("installed");
    await expect(page.getByTestId("base-cards")).toContainText("Purchasing");
    await expect(page.getByTestId("base-cards")).toContainText("Expenses");
    await expect(page.getByTestId("nav-Purchase requests")).toBeVisible();
    await shot(page, "12-template-installed");
  });

  test("everything built is in the audit chain", async () => {
    await page.getByTestId("nav-audit").click();
    for (const action of ["base.create", "refset.create", "table.create", "field.create", "setting.update", "member.create", "template.install"]) {
      await expect(page.getByTestId("audit-entries")).toContainText(action);
    }
    await page.getByTestId("verify").click();
    await expect(page.getByTestId("verdict")).toContainText("Chain intact");
    await shot(page, "13-audit");
  });
});
