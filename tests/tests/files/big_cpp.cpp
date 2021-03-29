/**
 * This test file attempts to create the following situations:
 * - Multiple levels of lexical scoping.
 * - Those lexical blocks are sufficiently large that it's common for the
 *   block open to be off of the screen so that a "position:sticky" style
 *   display would be appropriate.
 * - Call structures potentially look interesting if you graph them.
 *
 * This is accomplished by:
 * - Many silly comment blocks.
 * - Tons of copy and pasting and search and replace.
 * - Panicking when trying to trying to come up with subject matter and deciding
 *   that cats and dogs work.
 * - Not using templates.
 */

#include <stdio.h>
#include <stdlib.h>

#include "big_header.h"
#include "subdir/header@with,many^strange~chars.h"
#include "atom_magic.h"

class GlobalContext {
  public:

  static bool decideBooleanTrait() {
    int rval = rand();

    // boop boop
    //
    // boop

    int midpointValue = RAND_MAX / 2;

    if (rval > midpointValue) {
      // beep
      //
      // beep beep beep
      //
      // beep
      //
      //      beep
      //
      //           beep
      return true;
    }

    // BEEP BEEP BEEP BEEP
    // BEEP           BEEP
    // BEEP           BEEP
    // BEEP           BEEP
    // BEEP           BEEP
    // BEEP           BEEP
    // BEEP           BEEP
    // BEEP           BEEP
    // BEEP           BEEP
    // BEEP BEEP BEEP BEEP

    return false;
  }

  static bool decideEnigmaticAnimalBooleanTrait() {
    return decideBooleanTrait();
  }

  static bool decideCatBooleanTrait() {
    return decideEnigmaticAnimalBooleanTrait();
  }

  static bool decideBestFriendBooleanTrait() {
    return decideBooleanTrait();
  }

  class LessGlobalContext {
    public:
    // BARK
    //     BARK
    //         BARK
    //             BARK
    //                 BARK
    //                     BARK
    //                         BARK
    //                             BARK
    //                                 BARK
    static bool
    decideWhetherToDecide() {
      // BEEP BEEP BEEP BEEP
      // BEEP           BEEP
      // BEEP           BEEP
      // BEEP           BEEP
      // BEEP           BEEP
      // BEEP           BEEP
      // BEEP           BEEP
      // BEEP           BEEP
      // BEEP           BEEP
      // BEEP BEEP BEEP BEEP
      return true;
    }

    // BARK
    //     BARK
    //         BARK
    //             BARK
    //                 BARK
    //                     BARK
    //                         BARK
    //                             BARK
    //                                 BARK
  };

  // BARK
  //     BARK
  //         BARK
  //             BARK
  //                 BARK
  //                     BARK
  //                         BARK
  //                             BARK
  //                                 BARK

  static bool decideDogBooleanTrait() {
    if (LessGlobalContext::decideWhetherToDecide()) {
      return decideBestFriendBooleanTrait();
    }

    return false;
  };
};

namespace outerNS {

#define HUMAN_HP 100

class Thing {
  protected:
  // Finally, an example class that could evolve into a MUD!
  //
  // Health
  // Points
  int mHP;

  // Existence
  // Points
  bool mDefunct;

  public:
  Thing(int baseHP)
  : mHP(baseHP)
  , mDefunct(false) {
    // bop.
  }

  void ignore();

  virtual void takeDamage(int damage) {
    mHP -= damage;

    if (damage <= 0) {
      mDefunct = true;
      damage = 0;
    }
  }
};

void Thing::ignore() {
  // ignore
  // i g n o r e
  // i  g  n  o  r  e
  // i   g   n   o   r   e
  // i    g    n    o    r    e
  // i   g   n   o   r   e
  // i    g    n    o    r    e
  // i   g   n   o   r   e
  // i  g  n  o  r  e
  // i g n o r e
}

class Human: public Thing {
  public:

  Human()
  : Thing(HUMAN_HP) {

  }
};

class Superhero : public Human {
  public:

  Superhero()
  : Human() {

  }

  void takeDamage(int damage) override {
    // nope
    //   nope
    //     nope!
    //   ...
    // Superheroes don't take damage.
  }
};

class Couch : public Thing {
  public:

  Couch(int couchHP=20)
  : Thing (couchHP) {
    Superhero superBob;
    WhatsYourVector<Superhero> victor(&superBob);

    Human bob;
    WhatsYourVector<Human> goodReferenceRight(&bob);

    victor.forwardDeclaredTemplateThingInlinedBelow(this);
    goodReferenceRight.forwardDeclaredTemplateThingInlinedBelow(this);
  }
};

class OuterCat : Thing {
  private:
  // Cat secrets!
  // The secrets of cats!
  // These cannot be known to humans.
  // Or anyone.
  // Except perhaps other cats.
  // Or perhaps not.
  // Or perhaps...
  //           ...
  //           ...
  //           ...
  //           ...not!

  bool mIsFriendly;
  bool mIsSecretlyUnfriendly;

  public:
  // These things can be known.
  // But they are methods.
  // So they're not really things you know.
  // Although there are getters.
  // Shoot, maybe those should be protected.
  // Okay, now they're protected.
  // Although you haven't read that far down yet.
  // The comments don't get better.

  OuterCat(bool beFriendly,
    // what gets position:sticky'd here do you suppose
    // and how long does it last?
    // ...
    // ...
    // hm.
    bool beSecretlyUnfriendly)
    // more hm.
    // yes, very hm.
    // hm hm hm.
  : Thing(9 * HUMAN_HP)
  , mIsFriendly(beFriendly)
  // Unknown.
  // ...
  // Okay, we can probably implement things now.
  , mIsSecretlyUnfriendly(beSecretlyUnfriendly) {
    // Meow
    //  Meow
    //   Meow
    //    Meow
    //     Meow
    //      Meow
    //       Meow
    //     Meow
    //    Meow
    //   Meow
    //    Meow
    //     Meow
    //      Meow
    //       Meow
    //        Meow
    //         Meow
    //          Meow
    //           Meow
    //            Meow
    //             Meow
    //              Meow Meow Meow Meow Meow Meow Meow
    //                                            Meow
    //                                             Meow
    //                                              Meow
    //                                             Meow
    //                                            Meow
    //                                           Meow
    //                                          Meow
    //                                         Meow
    //                                        Meow
    //                                       Meow
    //                                      Meow
    //                                     Meow
  }


  protected:
  // Sorta secret things.
  // Like, other cats know these things.
  // But still, not for humans.
  // Not now.
  // Not ever.
  // Unless some type of special cat x-ray is developed.
  // Boy, wouldn't that be a thing.
  // Humanity is come so far, and yet we don't have a specialized cat x-ray...
  // one capable of seeing into the true nature of a cat.

  bool isFriendlyCat() {
    return mIsFriendly;
  }

  bool isSecretlyUnfriendly() {
    //                                             Meow
    //                                              Meow
    //                                             Meow
    //                                            Meow
    //                                           Meow
    //                                          Meow
    //                                         Meow
    //                                        Meow
    //                                       Meow
    //                                      Meow
    //                                     Meow
    return mIsSecretlyUnfriendly;
  }

  bool isFriendlyIfNotCurrentlyVisible() {
    if (isSecretlyUnfriendly()) {
      return true;
    }

    return isFriendlyCat();
  }

  public:

  void meet(Human &human) {
    human.ignore();
  }

  /**
   * Something there is that doesn't love a couch.
   *
   * A cat.
   *
   * A cat doesn't love a couch.
   */
  void meet(Couch &couch) {
    shred(couch);

    if (!isFriendlyCat()) {
      // D
      //  E
      //   S
      //    T
      //     R
      //      O
      //       Y
      destroy(couch);
    } else if (isFriendlyIfNotCurrentlyVisible()) {
      // NO
      //
      // D
      //  E
      //   S
      //    T
      //     R
      //      O
      //       Y

      // do nothing
    } else {
      // D
      //  E
      //   S
      //    T
      //     R
      //      O
      //       Y
      destroy(couch);
    }
  }

  /**
   * Standard cat destruction.
   */
  void shred(Thing &thing) {
    thing.takeDamage(1);
  }

  /**
   * More thorough cat destruction.
   */
  void destroy(Thing &thing) {
    // s
    shred(thing);

    //  h
    shred(thing);

    //   r
    shred(thing);

    //    e
    shred(thing);

    //     d
    shred(thing);
  }
};

namespace innerNS {

class InnerCat {

};

namespace {

class AnonCat {

};

#ifdef DEBUG

class DebugAnonCat {

};

#else // not ifdef DEBUG

class NondebugAnonCat {

};

#endif

}; // end anonymous namespace

} // end namespace innerNS

} // end namespace outerNS


void i_was_declared_in_the_header() {
  // Perhaps there was a bug where the declaration might have been treated as a
  // definition and then, adding insult to injury, the range from this file was
  // exposed in the header.
}
