// Code holding mutexes (see mutex.h), for the dataflow frontend's lock
// regions and the lock-held: view: a worker whose mMutex guards its queue and
// status, held by guards, released by an unlock, proven held by a parameter
// (DispatchLockHeld's), and acquired after its monitor; and a static mutex.

#include "mutex.h"

namespace mozilla::dom {

class Queue {
 public:
  void Push(int aRunnable);
  int Pop();
};

void Wake();
void CallOut(int aStatus);

class Worker {
 public:
  void Dispatch(int aRunnable);
  void Notify();
  bool Close();
  int Status() MOZ_REQUIRES(mMutex);

 private:
  void DispatchLockHeld(int aRunnable, const MutexAutoLock& aProofOfLock);

  Mutex mMutex;
  Monitor mMonitor;
  int mQueueLength MOZ_GUARDED_BY(mMutex);
  int mStatus MOZ_GUARDED_BY(mMutex);
  Queue mQueue MOZ_GUARDED_BY(mMutex);
  int mUnguarded;
};

void Worker::Dispatch(int aRunnable) {
  MutexAutoLock lock(mMutex);
  mQueueLength = mQueueLength + 1;
  mQueue.Push(aRunnable);
  DispatchLockHeld(aRunnable, lock);
}

void Worker::DispatchLockHeld(int aRunnable,
                              const MutexAutoLock& aProofOfLock) {
  mStatus = aRunnable;
  Wake();
}

void Worker::Notify() {
  int status = 0;
  {
    MutexAutoLock lock(mMutex);
    status = mStatus;
    {
      MutexAutoUnlock unlock(mMutex);
      CallOut(status);
    }
    auto later = [this]() { mUnguarded = 1; };
    later();
  }
  mUnguarded = status;
}

bool Worker::Close() {
  MonitorAutoLock monitor(mMonitor);
  mMonitor.Notify();
  MutexAutoLock lock(mMutex);
  return mStatus != 0;
}

int Worker::Status() { return mStatus; }

static StaticMutex sCountMutex;
static int sCount;

void Increment() {
  StaticMutexAutoLock lock(sCountMutex);
  sCount = sCount + 1;
}

}  // namespace mozilla::dom
