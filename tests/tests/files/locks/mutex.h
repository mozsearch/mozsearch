// A stand-in for Gecko's mutexes and their scoped guards (xpcom/threads'
// Mutex.h, Monitor.h and DataMutex.h, and xpcom/base/StaticMutex.h), for the
// dataflow frontend's lock regions (tools/src/dataflow/locks.rs, which knows
// their classes) and the lock-held: view: a guard holds its mutex from its
// construction to the end of its block, and an unlock releases it there.

#ifndef locks_mutex_h
#define locks_mutex_h

// (Clang's thread-safety attributes in Gecko; nothing here.)
#define MOZ_GUARDED_BY(x)
#define MOZ_REQUIRES(...)
#define MOZ_NO_THREAD_SAFETY_ANALYSIS
#define MOZ_PUSH_IGNORE_THREAD_SAFETY
#define MOZ_POP_THREAD_SAFETY

namespace mozilla {

class Mutex {
 public:
  void Lock();
  void Unlock();
};

class StaticMutex {
 public:
  void Lock();
  void Unlock();
};

class Monitor {
 public:
  void Lock();
  void Unlock();
  void Notify();
};

namespace detail {

template <typename T>
class BaseAutoLock {
 public:
  explicit BaseAutoLock(T aLock) : mLock(aLock) { mLock.Lock(); }
  ~BaseAutoLock() { mLock.Unlock(); }

 private:
  T mLock;
};

template <typename T>
class BaseAutoUnlock {
 public:
  explicit BaseAutoUnlock(T aLock) : mLock(aLock) { mLock.Unlock(); }
  ~BaseAutoUnlock() { mLock.Lock(); }

 private:
  T mLock;
};

}  // namespace detail

typedef detail::BaseAutoLock<Mutex&> MutexAutoLock;
typedef detail::BaseAutoUnlock<Mutex&> MutexAutoUnlock;
typedef detail::BaseAutoLock<StaticMutex&> StaticMutexAutoLock;

template <typename T>
class MonitorAutoLockBase {
 public:
  explicit MonitorAutoLockBase(T& aMonitor) : mMonitor(&aMonitor) {
    mMonitor->Lock();
  }
  ~MonitorAutoLockBase() { mMonitor->Unlock(); }

 private:
  T* mMonitor;
};

using MonitorAutoLock = MonitorAutoLockBase<Monitor>;

// A value only reached through its mutex's guard, which `Lock()` returns
// (xpcom/threads/DataMutex.h): `auto lock = mData.Lock(); lock->mField`.
template <typename T, typename MutexType>
class DataMutexBase {
 public:
  template <typename V>
  class AutoLockBase {
   public:
    V* operator->() const { return &mOwner->mValue; }
    ~AutoLockBase() { mOwner->mMutex.Unlock(); }

   private:
    friend class DataMutexBase;
    explicit AutoLockBase(DataMutexBase* aOwner) : mOwner(aOwner) {
      mOwner->mMutex.Lock();
    }
    DataMutexBase* mOwner;
  };

  using AutoLock = AutoLockBase<T>;
  AutoLock Lock() { return AutoLock(this); }

 private:
  MutexType mMutex;
  T mValue;
};

template <typename T>
using DataMutex = DataMutexBase<T, Mutex>;

}  // namespace mozilla

#endif
