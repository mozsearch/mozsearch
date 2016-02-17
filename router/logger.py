import multiprocessing
import sys
import datetime
import os

lock = multiprocessing.Lock()

def log(msg, *args):
    now = datetime.datetime.now()
    pid = os.getpid()
    lock.acquire()
    print '%s/pid=%d - %s' % (str(now), pid, msg % args)
    sys.stdout.flush()
    lock.release()
    
