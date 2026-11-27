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
  agenttable [--port 8080] [--host 127.0.0.1] [--database URL] [--import FILE.json]
             [--data FILE.json] [--fresh] [--demo] [--open]

  --port      another port, if 8080 is taken
  --host      0.0.0.0 to reach it from other machines on your network (no real sign-in yet!)
  --database  where the workspace is kept. Default: sqlite://agenttable-data (SQLite files in
              the folder agenttable-data next to the program). A server works too:
                mysql://user:password@host:3306/mysql          (MySQL 8)
                postgres://user:password@host:5432/postgres    (PostgreSQL 14 or later)
              The server creates its own databases there (mt_control, mt_t_…), so the user
              needs the right to create databases.
  --import    put a workspace saved as JSON (an earlier version's agenttable-data.json) into
              a new database. An agenttable-data.json next to the program is carried over
              by itself the first time.
  --data      keep the workspace in a JSON file instead of a database (as earlier versions)
  --fresh     start over (the stored workspace is deleted)
  --demo      a new workspace starts with the purchasing sample installed
  --open      open the browser

Every change to records is stored as a numbered batch in the database's journal, next to
the records themselves. Two servers must not share one database: the one started last takes
it over, and the other one refuses further changes (it says so) instead of overwriting.

Build your own
--------------
A new workspace is empty: you are Alice Admin, who may do everything. Build a base the way
the end-to-end test does (screenshots build-01 … build-33 on the release page):

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
     header to rename a field, make it required (or required only when a condition holds:
     a Receipt when the Total is above 300 USD), edit its choices or delete it.
  8. "Process" on the claims table: states (Draft, Submitted waiting for approval and locked,
     Approved, Paid, Rejected), buttons (Submit, Pay — only for whoever may "expense.pay" —,
     Revise, Withdraw) and the approval: Max approves, finance reviews only when "Needs
     finance" is Yes, finance is told it can pay. Publish (the designer refuses designs that
     could strand a record).
  9. Access: a set "Employee" (read, create, update) for Mia on her own records; read for Max;
     read, review and pay (up to 10,000 USD) for the finance team.
 10. Sign in as Mia, enter and submit a claim; as Max approve it in Approvals; as Fiona review
     and pay it; as Mia read the inbox.
 11. Access can be narrowed further: a grant on a table may cover only some fields (or all but
     some) and run from / until a date. An access review lets someone other than the holder
     keep or revoke each grant of a base (a revocation needs a note); completing it revokes.
 12. "Share…" on a record makes a read-only link for people without an account: this record
     or the whole table, with fields left out and an expiry. It never shows more than its
     maker may see, never links to other tables, and stops working when withdrawn.
 13. Code lists keep every version ("show" a past version as it was). Tick "Changes need a
     second person's approval": from then on a change is proposed and applies only when
     someone else approves it — turning this off needs approval too.
 14. Settings can differ per base: set expense.manager_limit "for the base Expenses" and the
     formulas of that base follow; click a setting to see which value applies where, and why,
     and every change made to it.

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

Not built yet (shown as placeholders): automations, email/chat notifications, dashboards,
the AI assistant. Sign-in is a demo switcher until real identity arrives.

Licence: Apache-2.0. Source and progress: https://github.com/wmkrfjnwvb-svg/agentTable
