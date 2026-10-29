FROM buildpack-deps:noble-curl@sha256:3ab6b90f4548f9d46251032d848d90cbf963b2263d0772125caa2b22d4e8f0a5

# Freeze the package universe as well as the base image. The build receipt records
# every installed package/version and the downloaded repository index hashes.
RUN sed -i 's|http://archive.ubuntu.com/ubuntu/|https://snapshot.ubuntu.com/ubuntu/20261029T000000Z/|g; s|http://security.ubuntu.com/ubuntu/|https://snapshot.ubuntu.com/ubuntu/20261029T000000Z/|g' /etc/apt/sources.list.d/ubuntu.sources \
 && apt-get -o Acquire::Check-Valid-Until=false update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends build-essential flex bison ccache cmake ninja-build python3 git curl ca-certificates xz-utils time \
 && mkdir /build-environment \
 && dpkg-query -W -f='${Package}\t${Version}\t${Architecture}\n' > /build-environment/dpkg.tsv \
 && find /var/lib/apt/lists -maxdepth 1 -type f -exec sha256sum '{}' + > /build-environment/apt-index-sha256.txt

RUN curl -fL --retry 2 https://nodejs.org/dist/v24.11.1/node-v24.11.1-linux-x64.tar.xz -o /tmp/node.tar.xz \
 && echo '60e3b0a8500819514aca603487c254298cd776de0698d3cd08f11dba5b8289a8  /tmp/node.tar.xz' | sha256sum -c - \
 && tar -xJf /tmp/node.tar.xz --strip-components=1 -C /usr/local \
 && rm /tmp/node.tar.xz

ENV LANG=C.UTF-8 LC_ALL=C.UTF-8 TZ=UTC CC=gcc CXX=g++
WORKDIR /work
