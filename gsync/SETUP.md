# gsync setup: one Google sign-in for Punchable + your Google Docs

What you get: "Sign in with Google" on auth.punchable.io logs you into Punchable **and** lets gsync read your Google Docs and write the docs Punchable makes. About 15 minutes, once. Design: `gsync/GSYNC.md`.

Use **owner@example.com** for every Google step.

## A. Google Cloud (you, in a browser)
1. **Create a project.** Go to https://console.cloud.google.com/projectcreate, name it `Punchable`, and click **Create**. Make sure `Punchable` is selected in the top bar.
2. **Turn on the APIs.** Open each link and click **Enable**:
   - https://console.cloud.google.com/apis/library/drive.googleapis.com
   - https://console.cloud.google.com/apis/library/docs.googleapis.com
3. **Set up the consent screen** at https://console.cloud.google.com/auth/overview, then **Get started**.
   1. App name `Punchable`. Support email owner@example.com. Audience **External**. Contact email owner@example.com. Agree, then **Create**.
   2. **Data Access → Add or remove scopes**. Tick `openid`, `.../auth/userinfo.email` and `.../auth/userinfo.profile`. Under "Manually add scopes", paste `https://www.googleapis.com/auth/drive`, then click **Add to table → Update → Save**.
   3. **Audience → Publish app → Confirm.** The status must read **In production**. Don't submit for verification.
      - **Why:** while an External app is in "Testing", Google issues "a refresh token expiring in 7 days". Source: https://developers.google.com/identity/protocols/oauth2, "Refresh token expiration".
      - In production, the refresh token doesn't expire on that schedule.
      - Unverified is fine for one person. Google only caps unverified apps at "100 new users in total" (https://support.google.com/cloud/answer/7454865), and shows a warning screen (step 9).
4. **Create the client.** At https://console.cloud.google.com/auth/clients, click **Create client**.
   - Application type **Web application**, name `Punchable Keycloak`.
   - **Authorized redirect URIs → Add URI**: `https://auth.punchable.io/realms/punchable/broker/google/endpoint`
   - Click **Create**.
5. **Save the ID and secret.** Copy both values from the dialog (the secret is shown in full only once) into a new file, `~\.punchable\google-oauth.env`:
   ```
   GOOGLE_CLIENT_ID=<paste>.apps.googleusercontent.com
   GOOGLE_CLIENT_SECRET=<paste>
   ```
   Then make the file readable by you only:
   ```powershell
   icacls "$env:USERPROFILE\.punchable\google-oauth.env" /inheritance:r /grant:r "$env:USERNAME:F"
   ```
   Never paste these values into chat, git or an issue.

## B. Keycloak on the server (the main session runs these from the repo, after the infra commit)
6. Test the realm on a throwaway Keycloak (the production image), including the red run:
   ```
   infra/ship.sh test
   ```
7. Put the Google client into a k8s Secret. The file is read on your PC and sent over ssh stdin; it never lands in argv, git or logs:
   ```
   infra/ship.sh google-idp
   ```
8. Apply the realm. This adds the Google button, the link-only first login, and the `pk-gsync` client:
   ```
   infra/ship.sh keycloak
   ```

## C. Link Google to your Punchable account (you, once)
9. **Link the accounts.**
   1. Open https://auth.punchable.io/realms/punchable/account and sign in with your Punchable email and password.
   2. Go to **Account security → Linked accounts → Google → Link account**, and pick owner@example.com.
   3. Google shows **"Google hasn't verified this app"**. Click **Advanced → Go to Punchable (unsafe)**. It's your own app.
   4. Allow **See, edit, create and delete all your Google Drive files**.
   - **Why linking first:** your Google address differs from your Punchable email. Linking while signed in ties them together safely.
   - "Sign in with Google" only works for an already-linked or matching account. Nobody else can sign up through it.
   - From now on **Sign in with Google** on the Punchable login page signs you in. Google asks for consent each time, which is what keeps a fresh refresh token on file.

## D. Your PC
10. The venv already exists at `~\.venvs\gsync`. Run everything from the repo:
    ```powershell
    cd <repo>
    $env:PYTHONUTF8=1
    ~\.venvs\gsync\Scripts\python -m gsync status     # lists exactly what is still missing
    ```
11. Sign gsync in. A code appears and your browser opens; choose **Sign in with Google**:
    ```powershell
    ~\.venvs\gsync\Scripts\python -m gsync auth
    ```
12. Do a dry run first (reads only), then the real run:
    ```powershell
    ~\.venvs\gsync\Scripts\python -m gsync run --dry-run
    ~\.venvs\gsync\Scripts\python -m gsync run
    ```
    The local API must be running (`python api-dev/server.py`); `status` says so if it isn't.
13. Keep it running at logon (every 60 s):
    ```powershell
    powershell -ExecutionPolicy Bypass -File gsync\install_task.ps1
    ```
    The log is `~/punchable-data\db\punchable\logs\gsync.log`.

## Everyday
- **Undo a run's Punchable changes:** `python -m gsync undo <run_id>`. Run ids are in `status` and `comedy\kb\gsync\runs.jsonl`.
- **Conflicts** (the same joke edited in the app and in its doc) are listed by `status` and kept in `comedy\kb\gsync\conflicts.jsonl`. The doc's text wins, and the app text is kept there word for word.
- **"FAILED" on the Google token about once a week** means the consent screen is still in Testing: redo step 3.3, then step 11.

## Fallback (only if Keycloak is unavailable)
Make a second client of type **Desktop app** (step 4, but Desktop), download its JSON to `~\.punchable\google_client.json`, and run `python -m gsync auth --desktop`. Delete `google_token.json` to switch back.
