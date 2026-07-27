FROM php:8.4-fpm
RUN apt-get update \
 && apt-get install -y --no-install-recommends libpq-dev \
 && docker-php-ext-install pdo_pgsql opcache \
 && rm -rf /var/lib/apt/lists/*
COPY fpm-pool.conf /usr/local/etc/php-fpm.d/zz-bench.conf
COPY opcache.ini   /usr/local/etc/php/conf.d/zz-opcache.ini
