#!/usr/bin/python3

import os
import subprocess
import yaml
import shlex
import shutil

def runcmd(cmd, cwd='.', extra_env=None, shell=False):
    if extra_env:
        env = os.environ.copy()
        env.update(extra_env)
    else:
        env = None
    print(f'cd {cwd} && {cmd}')
    subprocess.check_call(cmd, cwd=cwd, env=env, shell=shell)

def run_integration_tests(y):
    print('Running integration tests')
    runcmd(shlex.split('cargo +stable build --all-features --verbose --example simple --example pollcatch-without-agent'),
           extra_env=y['jobs']['build-for-testing']['env'])
    shutil.copy('./target/debug/examples/simple', './tests/simple')
    shutil.copy('./target/debug/examples/pollcatch-without-agent', './tests/pollcatch-without-agent')
    shutil.copy('./decoder/target/debug/pollcatch-decoder', './tests/pollcatch-decoder')
    for action in y['jobs']['test']['steps']:
        if action.get('shell') == 'bash':
            runcmd(action['run'], shell=True, cwd=action.get('working-directory', '.'))

def main():
    y = yaml.safe_load(open('.github/workflows/build.yml'))
    build = y['jobs']['build']
    for ek,ev in build['env'].items():
        os.environ[ek] = str(ev)

    # Decoder
    runcmd(['cargo', f'+nightly', 'fmt', '--all'], cwd='./decoder')
    runcmd(['cargo', f'+nightly', 'clippy', '--workspace', '--all-features', '--', '-D', 'warnings'], cwd='./decoder')
    runcmd(['cargo', '+nightly', 'test'], cwd='./decoder')
    runcmd(['cargo', '+nightly', 'build'], cwd='./decoder')

    # Main
    runcmd(['cargo', f'+nightly', 'fmt', '--all'])
    runcmd(['cargo', f'+nightly', 'clippy', '--workspace', '--all-features', '--', '-D', 'warnings'])
    for toolchain in build['strategy']['matrix']['toolchain']:
        for flags in build['strategy']['matrix']['flags']:
            runcmd(['cargo', f'+{toolchain}', 'build', '--verbose'] + shlex.split(flags))
            runcmd(['cargo', f'+{toolchain}', 'test', '--verbose'] + shlex.split(flags))
    runcmd(['cargo', '+nightly', 'doc', '--verbose', '--no-deps'], extra_env={'RUSTDOCFLAGS': '-D warnings --cfg docsrs'})

    run_integration_tests(y)

    print('All tests successful!')

if __name__ == '__main__':
    main()
