#!/bin/sh
# Receive one staged release on the Aliyun mirror, then republish both the
# launcher and boxpigma channel feeds.
set -eu
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
tar --no-same-owner -C /var/www/ferriso-updates -xzf -
/usr/local/sbin/ferriso-update-sync --local
/usr/local/sbin/boxpigma-update-sync --root /var/www/ferriso-updates
