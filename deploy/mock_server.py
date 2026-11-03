#!/usr/bin/env python3
"""Mock OAuth + usage server to test VibeRooms without touching real accounts.

Simulates Anthropic (Claude) and OpenAI (ChatGPT/Codex): access tokens valid 90 s, strictly rotating
refresh tokens (the previous one is invalidated; reuse is rejected like OpenAI does).
Usage: python3 deploy/mock_server.py [port]   then point VR_CLAUDE_* / VR_CODEX_* at it.
"""
import base64, json, math, sys, time, urllib.parse, uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

T0 = time.time()
TOKENS = {}    # access -> (acct, exp)
REFRESH = {}   # acct -> refresh courant
ACCTS = {}     # code -> acct
N = [0]
PROFILES = {
    "perso": ("alex@example.com", "alex@example.com's Organization", "claude_max", "default_claude_max_20x"),
    "msds": ("alex@acme.example", "Acme Corp", "claude_team", "default_claude_max_5x"),
    "third": ("alex.freelance@example.net", "Freelance", "claude_pro", "default_claude_pro"),
}

def iso(ts):
    return time.strftime("%Y-%m-%dT%H:%M:%S+00:00", time.gmtime(ts))

def usage(acct):
    el = time.time() - T0
    base = {"perso": 18, "msds": 71, "third": 4}[acct]
    five = min(100, base + el / 30)  # +2 %/min
    reset5 = T0 + 3 * 3600 - (40 if acct == "msds" else 0) * 60
    week = {"perso": 22, "msds": 64, "third": 9}[acct] + el / 600
    return {
        "five_hour": {"utilization": round(five, 1), "resets_at": iso(reset5)} if acct != "third" else {"utilization": 0, "resets_at": None},
        "seven_day": {"utilization": round(week, 1), "resets_at": iso(T0 + 2.4 * 86400)},
        "seven_day_opus": {"utilization": round(week * 0.6, 1), "resets_at": iso(T0 + 2.4 * 86400)} if acct == "perso" else None,
        "seven_day_sonnet": {"utilization": round(week * 0.3, 1), "resets_at": iso(T0 + 2.4 * 86400)} if acct == "msds" else None,
        "seven_day_oauth_apps": None,
        "extra_usage": {"is_enabled": acct == "msds", "monthly_limit": 5000, "used_credits": 1240, "utilization": 24.8, "currency": "EUR"} if acct == "msds" else {"is_enabled": False},
    }

# ---------- OpenAI / Codex ----------
CODEX = {"gpt-plus": ("alex@example.com", "plus"), "gpt-pro": ("alex@acme.example", "pro")}
C_TOKENS = {}   # access -> (acct, exp)
C_REFRESH = {}  # acct -> current refresh token

def b64(d):
    return base64.urlsafe_b64encode(json.dumps(d).encode()).decode().rstrip("=")

def jwt(claims):
    return "e30." + b64(claims) + ".sig"

def codex_usage(acct):
    el = time.time() - T0
    used5 = {"gpt-plus": 35, "gpt-pro": 8}[acct] + el / 40
    return {
        "plan_type": CODEX[acct][1],
        "rate_limit": {
            "allowed": True, "limit_reached": False,
            "primary_window": {"used_percent": min(100, round(used5)), "limit_window_seconds": 18000, "reset_at": int(T0 + 2 * 3600)},
            "secondary_window": {"used_percent": {"gpt-plus": 48, "gpt-pro": 12}[acct], "limit_window_seconds": 604800, "reset_at": int(T0 + 4 * 86400)},
        },
        "additional_rate_limits": [
            {"limit_name": "GPT-5 Pro", "rate_limit": {"primary_window": {"used_percent": 20, "limit_window_seconds": 604800, "reset_at": int(T0 + 4 * 86400)}}}
        ] if acct == "gpt-pro" else [],
        "credits": {"has_credits": False, "unlimited": False, "balance": "0"},
    }

class H(BaseHTTPRequestHandler):
    def log_message(self, fmt, *a):
        sys.stderr.write("mock " + fmt % a + "\n")

    def reply(self, code, obj):
        b = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def issue(self, acct):
        N[0] += 1
        a, r = f"a-{acct}-{N[0]}", f"r-{acct}-{N[0]}"
        TOKENS[a] = (acct, time.time() + 90)
        REFRESH[acct] = r
        return {"access_token": a, "refresh_token": r, "expires_in": 90, "token_type": "Bearer", "scope": "user:profile"}

    def issue_codex(self, acct):
        N[0] += 1
        email, plan = CODEX[acct]
        access = jwt({"exp": int(time.time() + 90), "n": N[0]})
        refresh = f"cr-{acct}-{N[0]}"
        C_TOKENS[access] = (acct, time.time() + 90)
        C_REFRESH[acct] = refresh
        idt = jwt({"email": email, "sub": "u-" + acct, "https://api.openai.com/auth": {
            "chatgpt_account_id": "acct-" + acct, "chatgpt_plan_type": plan, "chatgpt_user_id": "user-" + acct}})
        return {"access_token": access, "refresh_token": refresh, "id_token": idt, "token_type": "Bearer"}

    def codex_token(self, raw):
        ctype = self.headers.get("Content-Type", "")
        if "x-www-form-urlencoded" in ctype:
            form = {k: v[0] for k, v in urllib.parse.parse_qs(raw.decode()).items()}
            acct = form.get("code", "")
            if form.get("grant_type") != "authorization_code" or acct not in CODEX or not form.get("code_verifier"):
                return self.reply(400, {"error": "invalid_grant"})
            if form.get("redirect_uri") != "http://localhost:1455/auth/callback":
                return self.reply(400, {"error": "invalid_redirect_uri"})
            return self.reply(200, self.issue_codex(acct))
        body = json.loads(raw or b"{}")
        if body.get("grant_type") == "refresh_token":
            rt = body.get("refresh_token")
            acct = next((k for k, v in C_REFRESH.items() if v == rt), None)
            if not acct:
                return self.reply(401, {"error": {"code": "refresh_token_reused", "message": "Your refresh token has already been used"}})
            out = self.issue_codex(acct)
            out.pop("id_token")  # real refreshes may omit it
            return self.reply(200, out)
        return self.reply(400, {"error": "unsupported_grant_type"})

    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        if self.path == "/oauth/token":
            return self.codex_token(raw)
        body = json.loads(raw or b"{}")
        if self.path != "/v1/oauth/token":
            return self.reply(404, {"error": "not_found"})
        if body.get("grant_type") == "authorization_code":
            acct = body.get("code", "")
            if acct not in PROFILES or not body.get("code_verifier"):
                return self.reply(400, {"error": "invalid_grant", "error_description": "bad code"})
            out = self.issue(acct)
            email, org, *_ = PROFILES[acct]
            out["account"] = {"uuid": str(uuid.uuid5(uuid.NAMESPACE_DNS, email)), "email_address": email}
            out["organization"] = {"uuid": str(uuid.uuid5(uuid.NAMESPACE_DNS, org)), "name": org}
            return self.reply(200, out)
        if body.get("grant_type") == "refresh_token":
            rt = body.get("refresh_token")
            acct = next((k for k, v in REFRESH.items() if v == rt), None)
            if not acct:
                return self.reply(400, {"error": "invalid_grant", "error_description": "Refresh token not found or invalid"})
            return self.reply(200, self.issue(acct))
        self.reply(400, {"error": "unsupported_grant_type"})

    def do_GET(self):
        auth = self.headers.get("Authorization", "")[7:]
        if self.path == "/backend-api/wham/usage":
            tok = C_TOKENS.get(auth)
            if not tok or tok[1] < time.time():
                return self.reply(401, {"detail": "token expired"})
            if self.headers.get("chatgpt-account-id") != "acct-" + tok[0]:
                return self.reply(403, {"detail": "account mismatch"})
            return self.reply(200, codex_usage(tok[0]))
        tok = TOKENS.get(auth)
        if not tok or tok[1] < time.time():
            return self.reply(401, {"type": "error", "error": {"type": "authentication_error", "message": "OAuth token has expired."}})
        if self.headers.get("anthropic-beta") != "oauth-2025-04-20":
            return self.reply(400, {"error": {"message": "missing beta"}})
        acct = tok[0]
        if self.path == "/api/oauth/usage":
            return self.reply(200, usage(acct))
        if self.path == "/api/oauth/profile":
            email, org, kind, tier = PROFILES[acct]
            return self.reply(200, {
                "account": {"email": email, "display_name": "Allan", "has_claude_max": kind == "claude_max", "has_claude_pro": kind == "claude_pro"},
                "organization": {"name": org, "organization_type": kind, "rate_limit_tier": tier, "uuid": str(uuid.uuid5(uuid.NAMESPACE_DNS, org))},
            })
        self.reply(404, {"error": "not_found"})

ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1]) if len(sys.argv) > 1 else 8794), H).serve_forever()
