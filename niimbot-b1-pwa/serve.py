import functools
import http.server
import sys
import urllib.request
from pathlib import Path

STOCK = "/firmware/B1_6.19.bin"
UPSTREAM = "https://fw.niim.blue/stable/B1/B1_6.19.bin"


class Handler(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path != STOCK:
            return super().do_GET()
        try:
            with urllib.request.urlopen(UPSTREAM, timeout=30) as upstream:
                body = upstream.read()
        except OSError as error:
            return self.send_error(502, f"fw.niim.blue: {error}")
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


port = int(sys.argv[1]) if len(sys.argv) > 1 else 8787
www = Path(__file__).parent / "www"
server = http.server.ThreadingHTTPServer(("", port), functools.partial(Handler, directory=www))
server.serve_forever()
