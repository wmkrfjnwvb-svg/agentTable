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
  agenttable [--port 8080] [--host 127.0.0.1] [--data agenttable-data.json] [--fresh] [--open]

  --port    another port, if 8080 is taken
  --host    0.0.0.0 to reach it from other machines on your network (no real sign-in yet!)
  --data    where the data is kept (created with the demo data on first start)
  --fresh   start over with the demo data
  --open    open the browser

What to try
-----------
The demo is a small purchasing process: suppliers, purchase requests and their lines.
Use "Signed in as" at the top right to become another person:

  Alice Admin        sees and changes everything
  Bob Buyer          sees and changes only his own requests; no bank details
  Carol Controller   reads all requests; may approve up to 10,000 (not her own)
  Dave Finance       sees bank details in clear and the audit log
  Gina Guest         a supplier contact: sees only her own company
  Purchasing assistant (AI agent) — acting for someone, sees exactly what they see

Things to look at: change a line's quantity and watch totals recompute across tables; add a
formula field (the editor checks it as you type); group by status; press "Approve…" on a
request as Carol; open "Access & explain"; verify the audit chain and simulate tampering.

Not built yet (shown as placeholders): approvals and lifecycles, automations,
notifications, dashboards, the AI assistant. Storage is one JSON file until the database
layer arrives; sign-in is a demo switcher until real identity arrives.

Licence: Apache-2.0. Source and progress: https://github.com/wmkrfjnwvb-svg/agentTable
