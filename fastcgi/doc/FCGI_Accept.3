NAME
     FCGI_Accept, FCGI_ToFILE, FCGI_ToFcgiStream
         - fcgi_stdio compatibility library

SYNOPSIS
     #include "fcgi_stdio.h"

     int
     FCGI_Accept(void);

     FILE *
     FCGI_ToFILE(FCGI_FILE *);

     FCGI_Stream *
     FCGI_ToFcgiStream(FCGI_FILE *);


DESCRIPTION
     The FCGI_Accept function accepts a new request from the HTTP server
     and creates a CGI-compatible execution environment for the request.

     If the application was invoked as a CGI program, the first
     call to FCGI_Accept is essentially a no-op and the second
     call returns -1.  This causes a correctly coded FastCGI Responder
     application to run a single request and exit, giving CGI
     behavior.

     If the application was invoked as a FastCGI server, the first
     call to FCGI_Accept indicates that the application has completed
     its initialization and is ready to accept its first request.
     Subsequent calls to FCGI_Accept indicate that the application has
     completed processing its current request and is ready to accept a
     new request.  An application can complete the current request
     without accepting a new one by calling FCGI_Finish(3); later, when
     ready to accept a new request, the application calls FCGI_Accept.

     In completing the current request, FCGI_Accept may detect
     errors, e.g. a broken pipe to a client who has disconnected
     early.  FCGI_Accept ignores such errors.  An application
     that wishes to handle such errors should explicitly call
     fclose(stderr), then fclose(stdout); an EOF return from
     either one indicates an error.

     If the environment variable FCGI_WEB_SERVER_ADDRS is set when
     FCGI_Accept is called, it should contain a comma-separated list
     of IP addresses.  Each IP address is written as four decimal
     numbers in the range [0..255] separated by decimal points.
     (nslookup(8) translates the more familiar symbolic IP hostname
     into this form.)  So one legal binding for this variable is

         FCGI_WEB_SERVER_ADDRS=199.170.183.28,199.170.183.71

     FCGI_Accept checks the peer IP address of each new connection for
     membership in the list.  If the check fails (including the
     possibility that the connection didn't use TCP/IP transport),
     FCGI_Accept closes the connection and accepts another one
     (without returning in between).

     After accepting a new request, FCGI_Accept assigns new values
     to the global variables stdin, stdout, stderr, and environ.
     After FCGI_Accept returns, these variables have the same
     interpretation as on entry to a CGI program.

     FCGI_Accept frees any storage allocated by the previous call
     to FCGI_Accept.  This has important consequences:

         DO NOT retain pointers to the environ array or any strings
         contained in it (e.g. to the result of calling getenv(3)),
         since these will be freed by the next call to FCGI_Finish or
         FCGI_Accept.

         DO NOT use setenv(3) or putenv(3) to modify the environ array
         created by FCGI_Accept, since this will either leak storage
         or cause the next call to FCGI_Finish or FCGI_Accept to free
         storage that should not be freed.

         If your application needs to use setenv or putenv to modify
         the environ array, it should follow this coding pattern:

             char **savedEnviron, **requestEnviron;
             int acceptStatus;

             savedEnviron = environ;
             acceptStatus = FCGI_Accept();
             requestEnviron = environ;
             environ = savedEnviron;
             if(acceptStatus >= 0 && !FCGX_IsCGI()) {
                 /*
                  * requestEnviron points to name-value pairs in
                  * storage allocated by FCGI_Accept.  OK to read,
                  * not OK to retain pointers -- make copies instead.
                  */
             }
             /*
              * OK to do setenv or putenv, but beware of storage leaks!
              */

     In addition to the standard CGI environment variables, the
     environment variable FCGI_ROLE is always set to the role
     of the current request.  The roles currently defined are
     RESPONDER, AUTHORIZER, and FILTER.

     In the FILTER role, the additional variables FCGI_DATA_LENGTH
     and FCGI_DATA_LAST_MOD are also defined.  See the manpage
     FCGI_StartFilterData(3) for complete information.

     The macros FCGI_ToFILE and FCGI_ToFcgiStream are provided
     to allow escape to native functions that use the types FILE or
     FCGI_Stream.  In the case of FILE, functions would have to
     be separately compiled, since fcgi_stdio.h replaces the standard
     FILE with FCGI_FILE.

     
RETURN VALUES
     0 for successful call, -1 for error (application should exit).

SEE ALSO
     FCGI_Finish(3)
     FCGI_StartFilterData(3)
     FCGI_SetExitStatus(3)
     cgi-fcgi(1)
     nslookup(8)

HISTORY
     Copyright (c) 1996 Open Market, Inc.
     See the file "LICENSE.TERMS" for information on usage and redistribution
     of this file, and for a DISCLAIMER OF ALL WARRANTIES.
     $Id: FCGI_Accept.3,v 1.1.1.1 1997/09/16 15:36:25 stanleyg Exp $
