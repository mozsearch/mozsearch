class ContextName {
};

ContextProto.prototype = {
  method() {
    this.callCallback((function () {
      let callbackFunc = aCallback => {
        callAnotherCallback(() => {
          var Obj = {
            method2() {
              var Obj2;
              this.method3 = () => {
                let name = new ContextName();
              };
            }
          };
        });
      };
    })());
  }
};
