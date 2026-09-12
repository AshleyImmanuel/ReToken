import urllib.request
import urllib.error
import json
import subprocess
import time
import sys

def run_test():
    print("Starting ReToken daemon for E2E test...")
    # Spawn the proxy
    proxy_proc = subprocess.Popen(
        ["cargo", "run", "--release", "-p", "cli", "--", "run"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE
    )

    # Wait for proxy to bind
    time.sleep(3)

    proxy_url = "http://127.0.0.1:8888/v1/messages"
    
    payload = {
        "model": "claude-3-5-sonnet-20241022",
        "messages": [
            {"role": "user", "content": "Hello, world!"}
        ]
    }
    
    data = json.dumps(payload).encode('utf-8')
    req = urllib.request.Request(proxy_url, data=data, headers={
        "Content-Type": "application/json",
        "x-api-key": "fake-key"
    }, method="POST")

    print(f"Sending mock payload to {proxy_url}...")
    try:
        # We expect an error since the fake-key is invalid and the upstream will reject it,
        # but the connection to the *proxy* should succeed, meaning the proxy is alive and routing.
        response = urllib.request.urlopen(req)
        print("Received success response:", response.read())
    except urllib.error.HTTPError as e:
        print(f"Proxy intercepted and routed correctly! Upstream returned: {e.code}")
        # 400 or 401 or 502 means ReToken successfully forwarded it and got a real upstream error
        if e.code in [400, 401, 502]:
            print("E2E Test Passed: Proxy is functioning and routing requests.")
        else:
            print("E2E Test Failed with unexpected HTTP code:", e.code)
            proxy_proc.kill()
            sys.exit(1)
    except urllib.error.URLError as e:
        print("E2E Test Failed: Could not connect to proxy. Is it running?", e)
        proxy_proc.kill()
        sys.exit(1)

    # Kill proxy
    proxy_proc.terminate()
    print("Test complete.")

if __name__ == "__main__":
    run_test()
