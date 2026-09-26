# Fabio — character and voice

Read this before writing any text a user will see: labels, empty states, errors,
confirmations, release notes, the README.

## Who Fabio is

Fabio is an alpine marmot who keeps watch over your databases.

Marmots live in burrows they dig carefully and keep tidy. They spend long hours
sitting in the sun on a warm rock, completely at ease. And one of them is always
on watch: when something is actually wrong, it gives **one clear whistle** and
the whole colony knows. It doesn't whistle at every cloud.

That's the whole personality:

| Marmot | Fabio |
|---|---|
| Sits calmly on the rock | The app is quiet. No badges, no popups, no "tips". |
| Keeps a tidy burrow | Your connections, tables and queries are where you left them. |
| One sentinel whistles once | A warning appears when it matters, says what and why, and isn't repeated. |
| Lives in a colony | Humans and agents share the burrow; everyone sees what everyone did. |
| Hibernates | Idle means idle: no background work, no memory creeping up. |
| Stores fat for winter | Fast because it remembers: schema cached, last query kept. |

## Principles

How Fabio should feel:

- **Typography does the work.** Hierarchy comes from weight and size, not from
  boxes, borders and icons.
- **Warm paper, one red.** Off-white background, warm greys, a single accent
  (`#D9534F`). Red means "this is the thing": the active item, the sort
  arrow, the one warning. Not decoration.
- **Restraint as a feature.** Every control earns its place. If a feature
  needs a manual, it's the wrong feature.
- **Keyboard first, mouse welcome.** Everything important has a shortcut, and
  the shortcut is shown where the action lives.
- **Small delights, never in the way.** One nice detail beats ten animations.

## How Fabio looks

- **Build:** round, compact, sturdy. A body like a small bean, short legs,
  and a head that flows straight into it with barely a neck.
- **Pose:** the sentinel. Upright on the hind legs, front paws held together
  at the chest, looking slightly off to one side. Alert, not anxious.
- **Colour:** warm chestnut-brown fur, a lighter cream muzzle and belly, dark
  nose, small rounded ears. The only red is the accent, used sparingly, e.g.
  a thin scarf or the rim of the burrow.
- **Style:** flat shapes, soft light, no outlines heavier than the shapes
  need. It must read as a marmot at 16 px.
- **Never:** cartoon eyes with shine highlights, open-mouth grins, a hard hat,
  a laptop, a database cylinder cliché on its own.

Where he appears: the app icon, the empty state, the About box, the README.
**Not** in errors, not in dialogs, not as a loading spinner. A mascot in an
error message is mocking you.

## How Fabio talks

Plain, short, specific, calm. A colleague who knows Postgres well and doesn't
need you to know that they know.

### Rules

1. **Say what happened, then what to do.** In that order, in one or two sentences.
2. **Numbers over adjectives.** "3,503 rows · 1.1 ms", not "Loaded quickly!"
3. **Use the database's own words.** Pass Postgres and SQLite error text
   through unchanged. Add context around it, never paraphrase it away.
4. **No exclamation marks.** Not for success, not for errors.
5. **Never blame.** "No password saved for this connection", not "You forgot
   the password".
6. **Warn once.** One sentence, the reason, the way out. Don't repeat it on
   every screen.
7. **Honest about uncertainty.** "~5.0M rows (estimate)" when that's what it
   is. Never present a guess as a fact.
8. **Marmot jokes: at most one per screen, never in an error, never in the UI
   chrome.** Empty states and the About box can smile. Everything else is plain.

### Examples

| Situation | ✅ Fabio | ❌ Not Fabio |
|---|---|---|
| Empty, no connection | Pick a connection, or drop a SQLite file here. | Welcome to Fabio! 🎉 Let's get started by connecting your first database! |
| Empty, connected | Pick a table. | Your data is waiting for you! |
| Counting a huge table | ~5.0M rows (estimate) | Calculating... |
| Too slow to count | too many rows to count quickly | Error: count failed |
| Server error | column "nme" does not exist | Oops! Something went wrong 😕 |
| Read-only block | This connection is read-only. Switch the tab to write mode to change data. | Permission denied |
| Delete connection | Delete “prod”? The saved password is removed too. | Are you sure? This action cannot be undone! |
| Agent ran a query | Agent ran 3 queries on chinook · 42 ms | 🤖 AI magic happened! |
| About box | Fabio keeps watch over your databases. He whistles once when something's wrong. | The ultimate next-gen database experience. |

### Words

| Use | Avoid |
|---|---|
| connection, table, row, query | data source, entity, record, request |
| read-only, write mode | safe mode, danger zone |
| estimate | approximately-ish, roughly around |
| agent | AI, assistant, copilot, magic |
| Delete, Save, Test, Cancel | Yes / No / OK |

## Sound

If Fabio ever makes a sound, it's a single short whistle, used only for the
one warning that matters (e.g. the agent tried to write). Off by default.
