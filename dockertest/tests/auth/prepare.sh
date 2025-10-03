#!/bin/bash

cat > /etc/ferron.kdl <<EOF
:80 {
  user "test" "$(echo -n test | argon2 $(dd if=/dev/urandom bs=16 count=1 2>/dev/null | base64 -w 0 | tr -d '=') -id -e )"
  status 401 realm="HTTP authentication test" users="test"
  root "/var/www/ferron"
}
EOF
