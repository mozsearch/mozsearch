// Code holding mutexes (see mutex.h), for the dataflow frontend's lock
// regions and the lock-held: view: a worker whose mMutex guards its queue and
// status (by its annotations), held by guards, released by an unlock, proven
// held by a parameter (DispatchLockHeld's), required held (Status), acquired
// after its monitor, and held as a reporter's reference to it; a guarded
// field read without it (UnsafePeek), and in a method without the analysis
// (Snapshot) and a lambda requiring it (Drain's); a static mutex; a cache's
// counters in a DataMutex, held by the guard its Lock() returns; and calls
// with it held: a function's (Note), a logging macro's (WORKER_LOG's Log),
// and an interface's with more implementations than calls are followed to
// (Target::Run).  And a retrier whose lock a lambda capturing `this` holds
// (where it first uses `this`, which clang defines the capture's field at),
// whose guarded field is set where the analysis is ignored (Init's pragmas),
// and whose sink is a file's or a gtest's fake's (gtest/FakeSink.cpp's, test
// code, which Retry doesn't run).

#include "mutex.h"
#include "sink.h"

namespace mozilla::dom {

class Queue {
 public:
  void Push(int aRunnable);
  int Pop();
};

void Wake();
void CallOut(int aStatus);
void Note(int aValue) { (void)aValue; }
void Log(int aLevel) { (void)aLevel; }
#define WORKER_LOG(level) Log(level)

class Target {
 public:
  virtual void Run() = 0;
};
class TargetA : public Target {
 public:
  void Run() override {}
};
class TargetB : public Target {
 public:
  void Run() override {}
};
class TargetC : public Target {
 public:
  void Run() override {}
};
class TargetD : public Target {
 public:
  void Run() override {}
};
class TargetE : public Target {
 public:
  void Run() override {}
};

class Worker {
 public:
  void Dispatch(int aRunnable);
  void Notify();
  bool Close();
  int Status() MOZ_REQUIRES(mMutex);
  int UnsafePeek();
  int Snapshot();
  void Drain();

 private:
  friend class Reporter;

  void DispatchLockHeld(int aRunnable, const MutexAutoLock& aProofOfLock);

  Mutex mMutex;
  Monitor mMonitor;
  int mQueueLength MOZ_GUARDED_BY(mMutex);
  int mStatus MOZ_GUARDED_BY(mMutex);
  Queue mQueue MOZ_GUARDED_BY(mMutex);
  int mDeferred
      MOZ_GUARDED_BY(mMutex);
  Target* mTarget;
  int mUnguarded;
};

void Worker::Dispatch(int aRunnable) {
  MutexAutoLock lock(mMutex);
  mQueueLength = mQueueLength + 1;
  mDeferred = 0;
  mQueue.Push(aRunnable);
  Note(aRunnable);
  WORKER_LOG(aRunnable);
  mTarget->Run();
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

int Worker::UnsafePeek() { return mStatus; }

int Worker::Snapshot() MOZ_NO_THREAD_SAFETY_ANALYSIS { return mQueueLength; }

void Worker::Drain() {
  auto pending = [this]() MOZ_REQUIRES(mMutex) { return mQueueLength; };
  MutexAutoLock lock(mMutex);
  pending();
}

class Reporter {
 public:
  explicit Reporter(Worker* aWorker) : mMutex(aWorker->mMutex) {}
  void Report();

 private:
  Mutex& mMutex;
  int mReports = 0;
};

void Reporter::Report() {
  MutexAutoLock lock(mMutex);
  mReports = mReports + 1;
}

static StaticMutex sCountMutex;
static int sCount;

void Increment() {
  StaticMutexAutoLock lock(sCountMutex);
  sCount = sCount + 1;
}

struct Counters {
  int mHits = 0;
};

class Cache {
 public:
  void Hit();

 private:
  DataMutex<Counters> mCounters;
};

void Cache::Hit() {
  auto counters = mCounters.Lock();
  counters->mHits = counters->mHits + 1;
}

class FileSink : public Sink {
 public:
  void Write() override {}
};

class Retrier {
 public:
  void Init(int aInfo);
  void Retry();

 private:
  Mutex mLock;
  int mInfo MOZ_GUARDED_BY(mLock);
  Sink* mSink;
};

void Retrier::Init(int aInfo) {
  MOZ_PUSH_IGNORE_THREAD_SAFETY
  mInfo = aInfo;
  MOZ_POP_THREAD_SAFETY
}

void Retrier::Retry() {
  auto reset = [&]() {
    {
      MutexAutoLock lock(mLock);
      mInfo = 0;
    }
  };
  reset();
  MutexAutoLock lock(mLock);
  mSink->Write();
}

}  // namespace mozilla::dom
