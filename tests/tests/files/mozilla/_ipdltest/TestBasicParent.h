/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this file,
 * You can obtain one at http://mozilla.org/MPL/2.0/. */

#ifndef mozilla_ipdltest_TestBasicParent_h
#define mozilla_ipdltest_TestBasicParent_h

#include "mozilla/_ipdltest/PTestBasicParent.h"

namespace mozilla::_ipdltest {

class TestBasicParent : public PTestBasicParent {
  NS_INLINE_DECL_THREADSAFE_REFCOUNTING(TestBasicParent, override)

 private:
  ~TestBasicParent() = default;
};

}  // namespace mozilla::_ipdltest

#endif  // mozilla_ipdltest_TestBasicParent_h
