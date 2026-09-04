# gramit

### AI actions, anywhere you work.

**Select anything → press `Ctrl+Alt+F` → tell Gramit what to do.**

Gramit is a keyboard-first AI assistant for **People who sit infront of computers and developers**. It works across your computer, so you can transform selected text or code without switching to ChatGPT, Claude, or another app.

```text
Tell Gramit what you want in terminal
   ↓
Ctrl + A
   ↓
Ctrl + Alt + F
   ↓
Result replaces the selection
```

### One shortcut. Anything you're working on.

**Code**

```text
// fix the race condition in this function
function processPayment() {
    ...
}
```

→ Gramit returns the fixed code.

**Prompts**

```text
make a login page
```

→ Gramit turns the rough idea into a detailed prompt for Claude, ChatGPT, Cursor, or another AI.

**Writing**

```text
tell ravi i am on leave tomorrow
```

→ Gramit turns it into a ready-to-send message or email.

**Grammar**

```text
Hey hr, wht are u donig ?
```

→ Turn rough buggy english into a fixed grammatically proffesional English.

### Built for the way developers and founders actually work

You constantly move between **code, terminals, GitHub, Slack, emails, documentation, customer feedback, and AI tools**.

Gramit removes the copy → paste → prompt → copy → paste loop.

**Select it. Tell Gramit what to do. Keep working.**

---

## What Gramit can do

Gramit currently supports four core modes:

| Mode      | What it does                                                          |
| --------- | --------------------------------------------------------------------- |
| `grammar` | Fixes grammar, spelling and punctuation without changing your wording |
| `write`   | Turns your instruction into the finished piece                        |
| `code`    | Writes or fixes code, including requests written as code comments     |
| `prompt`  | Turns rough requests into structured prompts for other AI tools       |

One mode is active at a time. Switch with:

```bash
gramit mode grammar
gramit mode write
gramit mode code
gramit mode prompt
```

The default shortcut is:

```text
Ctrl + Alt + F
```

On macOS:

```text
Ctrl + Option + F
```

## Installation

### Windows

Open PowerShell:

```powershell
irm https://raw.githubusercontent.com/JoeCelaster/gramit/main/install.ps1 | iex
```

### macOS

Open Terminal:

```bash
curl -fsSL https://raw.githubusercontent.com/JoeCelaster/gramit/main/install.sh | sh
```

### Linux

Open Terminal:

```bash
curl -fsSL https://raw.githubusercontent.com/JoeCelaster/gramit/main/install.sh | sh
```

After installation, run:

```bash
gramit setup
gramit start
gramit doctor --fix
```

---

## How to use

### 1. Start Gramit

```bash
gramit start
```

### 2. Choose Mode

```bash
- grammar
- write
- code
- prompt
```

### 2. Select text or code

```text
Ctrl + A
```

### 3. Press the shortcut

**Windows / Linux**

```text
Ctrl + Alt + F
```

**macOS**

```text
Ctrl + Option + F
```

### 4. Gramit replaces the selection

The selected text is sent to the configured AI backend and the result is pasted back in its place.

---

### Configure your hotkey

You can change the default hotkey anytime:

```bash
gramit config set hotkey <Key name>
```

For example:

```bash
gramit config set hotkey Ctrl+Alt+G
```

Then restart Gramit:

```bash
gramit restart
```

Your new hotkey will be used from then on.

### Other useful commands

```bash
gramit status
gramit mode
gramit version
gramit update
gramit doctor
```