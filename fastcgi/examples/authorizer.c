/*
 * tiny-authorizer.c --
 *
 * FastCGI example Authorizer program using fcgi_stdio library
 *
 * Copyright (c) 1996 Open Market, Inc.
 * See the file "LICENSE.TERMS" for information on usage and redistribution
 * of this file, and for a DISCLAIMER OF ALL WARRANTIES.
 *
 * $Id: authorizer.c,v 1.1 2001/06/19 15:30:02 robs Exp $
 */

#include "fcgi_stdio.h"
#include <stdlib.h>
#include <string.h>

int main(void)
{
    char *user, *password;

    user = getenv("USER");
    if (user == NULL) {
        user = "doe";
    }

    password = getenv("PASSWORD");
    if (password == NULL) {
        password = "xxxx";
    }

    while (FCGI_Accept() >= 0) {
        char *remoteUser, *remotePassword;

        remoteUser = getenv("REMOTE_USER");
        remotePassword = getenv("REMOTE_PASSWD");
        if ((remoteUser == NULL) || (remotePassword == NULL)
             || strcmp(remoteUser, user) || strcmp(remotePassword, password))
        {
             printf("Status: 401 Unauthorized\r\n"
                 "WWW-Authenticate: Basic realm=\"Test\"\r\n"
                 "\r\n");
        }
        else {
            char *processId = getenv("QUERY_STRING");
            if (processId == NULL || strlen(processId) == 0) {
                processId = "0";
        }
            printf("Status: 200 OK\r\n"
                "Variable-AUTH_TYPE: Basic\r\n"
                "Variable-REMOTE_PASSWD:\r\n"
                "Variable-PROCESS_ID: %s\r\n"
                "\r\n", processId);
        }
    }

    return 0;
}
