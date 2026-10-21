#!/usr/bin/env python3

# Credentials for the scripts here when they're run with an AWS SSO profile
# (ex: AWS_PROFILE=searchfox, after `aws sso login`).  boto3 exchanges the SSO
# session's token for role credentials in every process, so once the SSO
# session ends (hours after the login), the scripts fail with a
# TokenRetrievalError ("Token has expired and refresh failed"), while the aws
# CLI keeps working on the role credentials it cached at the login, which last
# longer.
#
# Importing this module, before using boto3, checks that boto3 can get
# credentials, and if it can't because of the SSO session, uses the aws CLI's
# (`aws configure export-credentials`) for the rest of the process; and if
# those have expired too, runs `aws sso login` if there's a terminal to log in
# from, or says to.  Elsewhere (ex: instances with roles, and Lambda, see
# build-lambda-indexer-start.sh), boto3's own credentials work, and this does
# nothing more.

import json
import os
import subprocess
import sys

import boto3
import botocore.exceptions

# The errors of an ended SSO session, or of having no credentials at all.
SSO_ERRORS = (
    botocore.exceptions.SSOError,
    botocore.exceptions.TokenRetrievalError,
    botocore.exceptions.UnauthorizedSSOTokenError,
    botocore.exceptions.NoCredentialsError,
)


def boto3_works():
    """Whether boto3's default session gets credentials (which it then keeps
    for the clients the script makes), or None if it fails for another reason
    than the SSO session, which the script can deal with as it would have."""
    boto3.setup_default_session()
    try:
        credentials = boto3.DEFAULT_SESSION.get_credentials()
        if credentials is None:
            return False
        credentials.get_frozen_credentials()
        return True
    except SSO_ERRORS:
        return False
    except Exception:
        return None


def cli_credentials():
    """The aws CLI's credentials, if it has any that work."""
    try:
        result = subprocess.run(
            ['aws', 'configure', 'export-credentials', '--format', 'process'],
            capture_output=True, text=True, timeout=120)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if result.returncode != 0:
        return None
    try:
        return json.loads(result.stdout)
    except ValueError:
        return None


def use(credentials):
    # (boto3 prefers these to the profile's, even with AWS_PROFILE set.)
    os.environ['AWS_ACCESS_KEY_ID'] = credentials['AccessKeyId']
    os.environ['AWS_SECRET_ACCESS_KEY'] = credentials['SecretAccessKey']
    if credentials.get('SessionToken'):
        os.environ['AWS_SESSION_TOKEN'] = credentials['SessionToken']
    boto3.setup_default_session()


def ensure():
    if boto3_works() is not False:
        return
    credentials = cli_credentials()
    if credentials is None and sys.stdin.isatty() and sys.stderr.isatty():
        print('The AWS SSO session has ended; logging in again with `aws sso login`.',
              file=sys.stderr)
        subprocess.run(['aws', 'sso', 'login'])
        if boto3_works() is not False:
            return
        credentials = cli_credentials()
    if credentials is None:
        sys.exit('No AWS credentials work; the SSO session has probably ended.  '
                 'Run `aws sso login` and try again.')
    print('(Using the aws CLI\'s credentials, as the AWS SSO session has ended.)',
          file=sys.stderr)
    use(credentials)


ensure()
