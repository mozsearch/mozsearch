/*
 * log-dump.c --
 *
 *	FastCGI example program to illustrate both an Authorizer and a
 *      Responder in a single application that are used to provide access
 *      to an ascii text file.  The intent of this application is to
 *      show the basic mechanics needed to display a log file for example
 *      though any ascii text file should work.
 *
 *
 * Copyright (c) 1996 Open Market, Inc.
 *
 * See the file "LICENSE.TERMS" for information on usage and redistribution
 * of this file, and for a DISCLAIMER OF ALL WARRANTIES.
 *
 */
#ifndef lint
static const char rcsid[] = "$Id: log-dump.c,v 1.5 2001/09/01 01:12:26 robs Exp $";
#endif /* not lint */

#include "fcgi_config.h"

#include <sys/types.h>
#include <stdlib.h>
#include <signal.h>
#include <string.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <errno.h>

#if defined __linux__
int kill(pid_t pid, int sig);
#endif

#ifdef HAVE_UNISTD_H
#include <unistd.h>
#endif

#include "fcgi_stdio.h"

static int successCount = 0;
static int failureCount = 0;

int main(void)
{
    char *queryString = NULL;
    char *rolePtr;
    char *authPtr;
    char *fileNamePtr = NULL;
    int fd, n, i, j;
    char temp[4096];
    char temp2[5000];

    while(FCGI_Accept() >= 0) {
        rolePtr = getenv("FCGI_ROLE");
	if(rolePtr == NULL) {
	    kill(getpid(), SIGQUIT);
	    exit(-1);
	}
	if(strstr(rolePtr, "AUTHORIZER")) {
	    queryString = getenv("QUERY_STRING");
	    if((queryString == NULL) ||
	       (strstr(queryString, "showme_the_log") == NULL)) {
	        failureCount++;
		printf("Status: 403 Forbidden\r\n"
		       "Content-type: text/html\r\n"
		       "\r\n"
		       "<title>FastCGI Forbidden!</title>"
		       "<h2>Access to URL: \"%s\" forbidden!</h2><p>"
		       "<h2>This is password protected and you "
		       "have not specified a valid password.</h2>"
		       "<p><h3>Total Failed Accesses: %d</h3>",
		       getenv("URL_PATH"), failureCount);
	    } else {
	        successCount++;
	        printf("Status: 200 OK\r\n"
                    "Variable-LOG_ACCESS: ACCESS_OK.%d\r\n"
                    "\r\n", successCount);
	    }
	    continue;
	}

	/*
	 * If we're being invoked as a RESPONDER, make sure that we've
	 * been granted access to return the file or that the file being
	 * requested is beyond access control (ie. per request file data).
	 */
	if(strstr(rolePtr, "RESPONDER")) {
	    authPtr = getenv("LOG_ACCESS");
	    if((authPtr == NULL) || (strstr(authPtr, "ACCESS_OK") == NULL)) {
	        failureCount++;
	        printf("Content-type: text/html\r\n\r\n"
		       "<h2>Access to log file \"%s\" denied</h2>"
		       "<p>Total Invalid Access Attempts: %d\r\n\r\n",
		       fileNamePtr, failureCount);
		continue;
	    }

	    fileNamePtr = getenv("LOG_FILE");
	    if(fileNamePtr == NULL || *fileNamePtr == '\0') {
	        failureCount++;
	        printf("Content-type: text/html\r\n\r\n"
		       "<h2>No file specified.</h2>>>"
		       "<p>Total Invalid Access Attempts: %d\r\n\r\n",
		       failureCount);
		continue;
	    }

	    fd = open(fileNamePtr, O_RDONLY, (S_IRGRP | S_IROTH | S_IRUSR));
	    if(fd < 0) {
	        printf("Content-type: text/html\r\n\r\n"
		       "<h2>File Error trying to access file \"%s\".</h2>"
		       "Error = %s\r\n\r\n", fileNamePtr, strerror(errno));
		continue;
	    }
	    printf("Content-type: text/html\r\n\r\n"
		   "<h2>Sending contents of file: %s</h2><p>"
		   "<h2>Successful Accesses: %d</h2>", fileNamePtr,
		   successCount);
	    while((n = read(fd, temp, 4096)) > 0) {
	        j = 0;
	        for(i = 0; i < n; i++) {
		    temp2[j] = temp[i];
		    if(temp[i] == '\n') {
		        strcpy(&temp2[j], "<p>");
			printf(temp2);
			j = 0;
		    } else {
		        j++;
		    }
		}
	    }
	    close(fd);
	    continue;
	}
    }

    exit(0);
}
