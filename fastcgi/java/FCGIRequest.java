/*
 * @(#)FCGIRequest.java
 *
 *  FastCGi compatibility package Interface
 *
 *  Copyright (c) 1996 Open Market, Inc.
 *
 * See the file "LICENSE.TERMS" for information on usage and redistribution
 * of this file, and for a DISCLAIMER OF ALL WARRANTIES.
 *
 * $Id: FCGIRequest.java,v 1.3 2000/03/21 12:12:26 robs Exp $
 */
package com.fastcgi;

import java.net.*;
import java.io.FileDescriptor;
import java.util.Properties;

public class FCGIRequest 
{
    private static final String RCSID = "$Id: FCGIRequest.java,v 1.3 2000/03/21 12:12:26 robs Exp $";

    /* This class has no methods. Right now we are single threaded
    * so there is only one request object at any given time which
    * is refrenced by an FCGIInterface class variable . All of this
    * object's data could just as easily be declared directly there.
    * When we thread, this will change, so we might as well use a
    * seperate class. In line with this thinking, though somewhat
    * more perversely, we kept the socket here.
    */
    /*
     * class variables
     */
    /*public static Socket  socket; */
    // same for all requests

    /*
     * instance variables
     */
    public Socket       socket;
    public boolean      isBeginProcessed;
    public int      requestID;
    public boolean      keepConnection;
    public int      role;
    public int      appStatus;
    public int      numWriters;
    public FCGIInputStream  inStream;
    public FCGIOutputStream outStream;
    public FCGIOutputStream errStream;
    public Properties   params;
}


