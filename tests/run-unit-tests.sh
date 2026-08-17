#!/usr/bin/env bash
#
# Copyright: 2020-2024 (c) Netdata Inc.
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Author: Austin S. Hemmelgarn <austin@netdata.cloud>
#
# shellcheck disable=SC2230

install_netdata() {
  echo "Installing Netdata"

  NETDATA_CMAKE_OPTIONS="-DCMAKE_BUILD_TYPE=Debug -DENABLE_ADDRESS_SANITIZER=On" \
  fakeroot ./netdata-installer.sh \
    --install-prefix "$HOME" \
    --dont-wait \
    --dont-start-it \
    --disable-lto
}

c_unit_tests() {
  echo "Running C code unit tests"

  ASAN_OPTIONS=detect_leaks=0 \
  "$HOME"/netdata/usr/sbin/netdata -W unittest
}

system_info_unit_test() {
  echo "Running system-info shell tests"
  /bin/sh "$(dirname "$0")/system-info-test.sh"
}

system_info_windows_unit_test() {
  echo "Running Windows system-info source/stub tests"
  python3 "$(dirname "$0")/system-info-windows-test.py"
}

kickstart_path_unit_test() {
  echo "Running kickstart path shell tests"
  /bin/sh "$(dirname "$0")/kickstart-path-sanitizer-test.sh"
}

stream_replication_query_test() {
  echo "Running stream replication query component test"

  local build_dir="${NETDATA_BUILD_DIR:-./build}"
  local config_h="${build_dir}/config.h"

  if [[ ! -r "${config_h}" ]]; then
    echo >&2 "Cannot read CMake configuration: ${config_h}"
    return 1
  fi

  if ! grep -Fqx '#define OS_LINUX' "${config_h}" ||
     ! grep -Fqx '#define ENABLE_DBENGINE' "${config_h}"; then
    echo "Skipping stream replication query component test (requires Linux with DBENGINE)"
    return 0
  fi

  cmake --build "${build_dir}" --target stream-replication-query-test || return 1
  "${build_dir}"/stream-replication-query-test
}

spawn_server_unit_tests() {
  # Shared with the macOS CI job, which builds netdata and then runs only this suite.
  bash "$(dirname "$0")/spawn-server-tests.sh"
}

acl_tests() {
  echo "Running ACL and bearer protection shell tests"
  # The build does not install developer test scripts, so render acl.sh from its
  # template against the install made above.
  acl_script="$(mktemp)" || return 1
  sed -e "s|@varlibdir_POST@|$HOME/netdata/var/lib/netdata|g" \
      -e "s|@sbindir_POST@|$HOME/netdata/usr/sbin|g" \
      "$(dirname "$0")/acls/acl.sh.in" > "$acl_script" || return 1
  if grep -q '@[A-Za-z_]*@' "$acl_script"; then
    echo "acl.sh.in has a placeholder this runner does not substitute" >&2
    return 1
  fi
  # acl.sh reads its netdata.cfg templates from the working directory
  (cd "$(dirname "$0")/acls" &&
    ASAN_OPTIONS=detect_leaks=0 \
    bash "$acl_script")
  rc=$?
  rm -f "$acl_script"
  return $rc
}

install_netdata || exit 1

c_unit_tests || exit 1

system_info_unit_test || exit 1

system_info_windows_unit_test || exit 1

kickstart_path_unit_test || exit 1

stream_replication_query_test || exit 1

spawn_server_unit_tests || exit 1

acl_tests || exit 1
