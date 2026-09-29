# OPcache is part of PHP itself from 8.5 on, so only pdo_pgsql is built here;
# zz-opcache.ini below still configures it.
FROM docker.io/library/php:8.5-fpm
RUN apt-get update \
 && apt-get install -y --no-install-recommends libpq-dev \
 && docker-php-ext-install pdo_pgsql \
 && rm -rf /var/lib/apt/lists/*
COPY fpm-pool.conf /usr/local/etc/php-fpm.d/zz-bench.conf
COPY opcache.ini   /usr/local/etc/php/conf.d/zz-opcache.ini
