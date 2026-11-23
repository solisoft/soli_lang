"""The checks of auth_races_solidb.sh: each single-use value, four requests at once."""
import base64
import hashlib
import http.cookiejar
import json
import os
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

SOLI, APP, BASE = sys.argv[1], sys.argv[2], sys.argv[3]
PASSWORD = "correct horse battery"
CALLBACK = "https://app.example/callback"
DEVICE_GRANT = "urn:ietf:params:oauth:grant-type:device_code"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def opener(jar=None):
    handlers = [NoRedirect]
    if jar is not None:
        handlers.append(urllib.request.HTTPCookieProcessor(jar))
    return urllib.request.build_opener(*handlers)


PLAIN = opener()


def call(via, path, data=None, headers=None):
    request = urllib.request.Request(BASE + path, data=data, headers=headers or {})
    try:
        reply = via.open(request, timeout=30)
        return reply.status, dict(reply.headers), reply.read().decode()
    except urllib.error.HTTPError as error:
        return error.code, dict(error.headers), error.read().decode()


def post_json(via, path, body):
    headers = {"Content-Type": "application/json", "Origin": BASE}
    return call(via, path, json.dumps(body).encode(), headers)


def together(calls):
    gate = threading.Barrier(len(calls))
    out = [None] * len(calls)

    def run(index, one):
        gate.wait()
        out[index] = one()

    threads = [threading.Thread(target=run, args=(i, c)) for i, c in enumerate(calls)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    return out


def runner(code):
    done = subprocess.run([SOLI, "runner", "-e", code], cwd=APP, capture_output=True, text=True)
    lines = (done.stdout + done.stderr).strip().splitlines()
    return lines[-1] if lines else ""


failures = []


def check(name, good, detail):
    print(("ok   " if good else "FAIL ") + name, detail)
    if not good:
        failures.append(name)


# --- the first account: sign-ups sent together make one admin at most
for round_ in range(6):
    runner("User.all.each do |user| user.delete end")
    statuses = together([
        lambda i=i: post_json(PLAIN, "/signup", {"email": f"first{round_}x{i}@example.com", "password": PASSWORD})[0]
        for i in range(4)
    ])
    counts = runner('print("#{User.where({"role": "admin"}).count} #{User.count}")')
    check("first account", counts in ("0 0", "1 1") and all(s < 500 for s in statuses), (statuses, counts))

# --- a reset link completed together changes the password once
for round_ in range(6):
    runner("User.all.each do |user| user.delete end")
    post_json(PLAIN, "/signup", {"email": "ada@example.com", "password": PASSWORD})
    token = runner('print(User.find_by("email", "ada@example.com").start_password_reset)')
    passwords = [f"a new password number {round_} {i}" for i in range(4)]
    statuses = together([
        lambda p=p: post_json(PLAIN, "/password/update", {"token": token, "password": p, "password_confirmation": p})[0]
        for p in passwords
    ])
    logins = [post_json(PLAIN, "/login", {"email": "ada@example.com", "password": p})[0] for p in passwords]
    good = statuses.count(302) == 1 and [s == 302 for s in statuses] == [s == 302 for s in logins]
    check("reset link", good and all(s < 500 for s in statuses), (statuses, logins))

# --- a passkey challenge is spent once (six processes)
for round_ in range(3):
    answers = together([
        lambda: runner(f'print(WebauthnChallenge.spend("race-{round_}"))') for _ in range(6)
    ])
    check("passkey challenge", answers.count("true") == 1, answers)

# --- the OpenID Connect provider
runner("User.all.each do |user| user.delete end")
jar = http.cookiejar.CookieJar()
browser = opener(jar)
post_json(browser, "/signup", {"email": "ada@example.com", "password": PASSWORD})
registered = runner(
    'result = OauthClient.register("Races", ["' + CALLBACK + '"], {"skip_consent": true, '
    '"scopes": ["openid", "email", "offline_access"], '
    '"grant_types": ["authorization_code", "refresh_token", "' + DEVICE_GRANT + '"]})\n'
    'print("#{result["client_id"]} #{result["client_secret"]}")'
)
client_id, client_secret = registered.split()
BASIC = "Basic " + base64.b64encode(f"{client_id}:{client_secret}".encode()).decode()


def token(params):
    headers = {"Content-Type": "application/x-www-form-urlencoded", "Authorization": BASIC}
    status, _, body = call(PLAIN, "/oauth/token", urllib.parse.urlencode(params).encode(), headers)
    try:
        return status, json.loads(body)
    except ValueError:
        return status, {}


def new_code():
    verifier = base64.urlsafe_b64encode(os.urandom(40)).decode().rstrip("=")
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip("=")
    query = urllib.parse.urlencode({
        "response_type": "code", "client_id": client_id, "redirect_uri": CALLBACK,
        "scope": "openid email offline_access", "state": "s", "nonce": "n",
        "code_challenge": challenge, "code_challenge_method": "S256",
    })
    status, headers, body = call(browser, "/oauth/authorize?" + query)
    location = headers.get("Location") or headers.get("location") or ""
    code = urllib.parse.parse_qs(urllib.parse.urlparse(location).query).get("code", [None])[0]
    assert code, (status, location, body[:300])
    return code, verifier


for round_ in range(6):
    code, verifier = new_code()
    params = {"grant_type": "authorization_code", "code": code, "redirect_uri": CALLBACK, "code_verifier": verifier}
    answers = together([lambda: token(params) for _ in range(4)])
    statuses = [a[0] for a in answers]
    # A code shown twice revokes what it issued: the winner's refresh token is dead.
    after = [
        token({"grant_type": "refresh_token", "refresh_token": a[1]["refresh_token"]})[0]
        for a in answers if a[0] == 200
    ]
    good = statuses.count(200) <= 1 and all(s < 500 for s in statuses) and all(s != 200 for s in after)
    check("authorization code", good, (statuses, after))

for round_ in range(6):
    code, verifier = new_code()
    status, first = token({
        "grant_type": "authorization_code", "code": code, "redirect_uri": CALLBACK, "code_verifier": verifier,
    })
    assert status == 200, (status, first)
    refresh = first["refresh_token"]
    answers = together([lambda: token({"grant_type": "refresh_token", "refresh_token": refresh}) for _ in range(4)])
    statuses = [a[0] for a in answers]
    # The retired token shown again revokes the family.
    after = [
        token({"grant_type": "refresh_token", "refresh_token": a[1]["refresh_token"]})[0]
        for a in answers if a[0] == 200
    ]
    good = statuses.count(200) <= 1 and all(s < 500 for s in statuses) and all(s != 200 for s in after)
    check("refresh token", good, (statuses, after))

for round_ in range(3):
    headers = {"Content-Type": "application/x-www-form-urlencoded", "Authorization": BASIC}
    asked = urllib.parse.urlencode({"client_id": client_id, "scope": "openid email offline_access"}).encode()
    status, _, body = call(PLAIN, "/oauth/device_authorization", asked, headers)
    assert status == 200, (status, body)
    device = json.loads(body)
    approvals = together([
        lambda: post_json(browser, "/oauth/device", {"user_code": device["user_code"], "approve": "1"})
        for _ in range(4)
    ])
    approved = sum(1 for s, _, b in approvals if s == 200 and "already used" not in b and "Too many" not in b)
    time.sleep(device.get("interval", 5) + 1)
    poll = {"grant_type": DEVICE_GRANT, "device_code": device["device_code"], "client_id": client_id}
    statuses = [a[0] for a in together([lambda: token(poll) for _ in range(4)])]
    time.sleep(device.get("interval", 5) * 3 + 1)
    late = token(poll)[0]
    good = approved == 1 and statuses.count(200) + (late == 200) == 1 and all(s < 500 for s in statuses)
    check("device code", good, (approved, statuses, late))

print("ALL OK" if not failures else "FAILURES: " + ", ".join(sorted(set(failures))))
sys.exit(1 if failures else 0)
