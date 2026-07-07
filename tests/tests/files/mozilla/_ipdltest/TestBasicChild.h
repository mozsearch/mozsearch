/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this file,
 * You can obtain one at http://mozilla.org/MPL/2.0/. */

#ifndef mozilla_ipdltest_TestBasicChild_h
#define mozilla_ipdltest_TestBasicChild_h

#include "mozilla/_ipdltest/PTestBasicChild.h"

namespace mozilla::_ipdltest {

class TestBasicChild : public PTestBasicChild {
  NS_INLINE_DECL_THREADSAFE_REFCOUNTING(TestBasicChild, override)

 public:
  mozilla::ipc::IPCResult RecvHello();

 private:
  ~TestBasicChild() = default;
};

}  // namespace mozilla::_ipdltest

#endif  // mozilla_ipdltest_TestBasicChild_h
