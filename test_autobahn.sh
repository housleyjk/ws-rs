#!/bin/bash

GREEN="\033[0;32m"
RED="\033[0;31m"
NOCOLOR="\033[0m"

# helpers to pretty print messages
echoinfo() { printf "${GREEN}[INFO]  %s${NOCOLOR}\n" "$@" ; }
echoerr() { printf "${RED}[ERROR] %s${NOCOLOR}\n" "$@" 1>&2 ; }

# make sure the autobahn testsuite is installed. If not, exit.
check_install() {
    echoinfo "Making sure the autobahn testsuite is installed"

    if [ $(which wstest 2>/dev/null) ] ; then
        echoinfo "The autobahn testsuites is installed correctly"
    else
        echoerr "The autobahn testsuite is not installed"
        echoerr "See https://github.com/crossbario/autobahn-testsuite for installation instructions"
        exit 1
    fi
}

kill_server() {
    if [ ! -z ${SERVER_PID} ] ; then
        kill ${SERVER_PID}
        echoinfo "Killed wstest (pid = ${SERVER_PID})"
    fi
    SERVER_PID=
}

prepend_stdout() {
    while read line; do
        printf "${GREEN}[${1}]${NOCOLOR} %s\n" "$line" ;
    done
}

prepend_stderr() {
    while read line; do
        printf "${RED}[${1}]${NOCOLOR} %s\n" "$line" 1>&2
    done
}

run_wstest_server() {
    echoinfo "Starting the client autobahn testsuite"
    wstest -m fuzzingserver -s ./tests/fuzzingserver.json \
        2> >(prepend_stderr "wstest") \
        > >(prepend_stdout "wstest")

    if [ $? -eq 0 ] ; then
        echoinfo "Server exited successfully"
    else
        echoerr "Server exited with an error"
    fi
}

# Start the autobahn server, to test ws-rs as a websocket client
test_client() {
    echoinfo "Starting client test"

    trap kill_server EXIT TERM INT RETURN

    run_wstest_server &
    SERVER_PID=$!
    echoinfo "Testsuite started (server pid = ${SERVER_PID})"

    sleep 1  # give wstest some time to start
    check_server_is_running

    echoinfo "Running ws-rs client"
    cargo run --release --all-features --example autobahn-client | prepend_stdout "ws-rs" 2>&1

    echoinfo "Client test finished, killing the server"
}


run_wsrs_server() {
    echoinfo "Running ws-rs server"
    cargo run --release --all-features --example autobahn-server 2>&1 | prepend_stdout "ws-rs"
    if [ ${PIPESTATUS[0]} -eq 0 ] ; then
        echoinfo "Server exited successfully"
    else
        echoerr "Server exited with an error"
    fi
}

check_server_is_running() {
    if ps -p ${SERVER_PID} > /dev/null ; then
        echoinfo "server is running"
    else
        echoerr "server is not running"
        exit 1
    fi

}

# Start the autobahn client, to test ws-rs as a websocket server
test_server() {
    echoinfo "Starting server test"

    trap kill_server EXIT TERM INT RETURN
    run_wsrs_server &
    SERVER_PID=$!

    sleep 1  # give the server some time to start
    check_server_is_running

    # check whether the server started correctly

    echoinfo "Starting the autobahn server testsuite"
    wstest -m fuzzingclient -s ./tests/fuzzingclient.json \
        2> >(prepend_stderr "wstest") \
        > >(prepend_stdout "wstest")

    echoinfo "Server test finished, killing the server"
}

usage() {
cat <<EOF
Usage:

    ./test_autobahn.sh COMPONENT [-c] [-d]
    ./test_autobahn.sh [-h]

Run the autobahn testsuite for the given component. When running on Travis, if
the autobahntestsuite is not installed, it is installed first.

Arguments:

    COMPONENT can be either "client" or "server"

Options:

    -h print this help message and exit
    -d enable bash debugs with \`set -x\`
EOF
}

main() {
    local client=
    local server=

    while getopts "cdhs" option ; do
        case ${option} in
            c) client=true ;;
            s) server=true ;;
            h) usage ; exit 0 ;;
            d) set -x ;;
        esac
    done

    # If neither -c or -s was specified, just print a usage message
    if [ -z ${client} ] && [ -z ${server} ] ; then
        usage
        exit 1
    fi

    check_install
    export RUST_BACKTRACE=1
    export RUST_LOG="ws=debug"
    [ ! -z ${client} ] && test_client
    [ ! -z ${server} ] && test_server

    exit 0
}

trap "exit 1" ERR
main $@
