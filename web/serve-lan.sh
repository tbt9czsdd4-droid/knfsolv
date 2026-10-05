#!/bin/sh
# Serves the browser build over HTTPS for phones on the same network.
#
# iOS 27 runs plain-HTTP pages from LAN addresses without its JavaScript and WebAssembly
# compiler, which makes the solver about 35x slower. Loopback is exempt, so this only matters
# for other devices. The certificate is self-signed: Safari warns once; continue with
# "Details einblenden" -> "diese Website besuchen".
#
# Usage: ./serve-lan.sh [lan-ip]
set -eu
cd "$(dirname "$0")"
ip=${1:-$(hostname -I | awk '{print $1}')}
cert=.tls/cert.pem
key=.tls/key.pem
if ! { [ -f "$cert" ] && openssl x509 -in "$cert" -noout -checkip "$ip" >/dev/null &&
    openssl x509 -in "$cert" -noout -checkend 604800 >/dev/null; }; then
    mkdir -p .tls
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 365 \
        -subj "/CN=Kniffel dev $ip" \
        -addext "subjectAltName=IP:$ip,IP:127.0.0.1,DNS:localhost" \
        -addext "extendedKeyUsage=serverAuth" \
        -keyout "$key" -out "$cert" 2>/dev/null
fi
echo "On the phone, open https://$ip:8443"
exec trunk serve --release --address 0.0.0.0 --port 8443 \
    --tls-key-path "$key" --tls-cert-path "$cert"
