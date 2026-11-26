agenttable — end-to-end preview build
=====================================

This package is the whole product as it stands today: one server program with the web app
built in. You run it on your computer and use it in your browser. Nothing is installed and
nothing is sent anywhere; everything stays in one data file next to the program.

Start
-----
  Windows:  double-click start.bat
  macOS:    double-click start.command  (first time: right-click → Open, because the
            program is not signed yet; or run `xattr -dr com.apple.quarantine .` in this folder)
  Linux:    ./start.sh

The browser opens at http://127.0.0.1:8080. Stop the server with Ctrl+C in its window.

Command line
------------
  agenttable [--port 8080] [--host 127.0.0.1] [--data agenttable-data.json] [--fresh] [--demo] [--open]

  --port    another port, if 8080 is taken
  --host    0.0.0.0 to reach it from other machines on your network (no real sign-in yet!)
  --data    where the data is kept (created on first start)
  --fresh   start over (the data file is replaced)
  --demo    a new workspace starts with the purchasing sample installed
  --open    open the browser

Build your own
--------------
A new workspace is empty: you are Alice Admin, who may do everything. Build a base the way
the end-to-end test does (screenshots build-01 … build-13 on the release page):

  1. Home → "Start from scratch": create a base, e.g. "Expenses".
  2. Code lists: a list of expense categories (a tree: Travel › Flights, Hotels; Meals).
  3. Settings: expense.manager_limit = 1000 USD (formulas read it with PARAM).
  4. In the base: tables "Expense claims" (named by Purpose) and "Expense lines".
  5. "+ Field" on each table: an auto number EXP-{YYYY}-{seq:4}, a Claimant (person, role
     requester), Date, Category (code list), Amount (currency, required), a two-way link
     from lines to their claim, a roll-up Total = SUM({Lines}.{Amount}), and a formula
     {Total} > PARAM("expense.manager_limit").
  6. People & groups: add the people who will use it.
  7. Enter a claim and its lines; watch the total and the formula follow. Click a column
     header to rename a field, make it required, edit its choices or delete it.

Or install the purchasing sample from the template gallery on the Home page (or start with
--demo): suppliers, purchase requests with lines, cost centres, approvals routed by limits
and a CFO step, and seven people with different access. Use "Signed in as" at the top right
to become another person:

  Alice Admin        sees and changes everything
  Bob Buyer          sees and changes only his own requests; no bank details
  Carol Controller   reads all requests; approves up to 10,000 (never her own)
  Dave Finance       approves up to 100,000, places orders, sees bank details and the audit log
  Erin CFO           approves requests above 25,000
  Gina Guest         a supplier contact: sees only her own company
  Purchasing assistant (AI agent) — acting for someone, sees exactly what they see

Not built yet (shown as placeholders): designing lifecycles, approvals and access rules on
screen (next), automations, email/chat notifications, dashboards, the AI assistant. Storage is
one JSON file until the database layer arrives; sign-in is a demo switcher until real
identity arrives.

Licence: Apache-2.0. Source and progress: https://github.com/wmkrfjnwvb-svg/agentTable
