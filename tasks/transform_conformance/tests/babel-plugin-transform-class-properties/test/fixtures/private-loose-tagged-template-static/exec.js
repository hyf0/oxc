class Foo {
  static #tag = function() { return this };

  static getReceiver() {
    return this.#tag`tagged template`;
  }
}

expect(Foo.getReceiver()).toBe(Foo);