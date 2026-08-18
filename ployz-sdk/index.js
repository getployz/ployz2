"use strict";

const native = require("./ployz-sdk.node");

class Client {
  constructor(inner) {
    this._inner = inner;
    this.runtime = {
      watch: (options = {}) => iterateWatch(() => inner.watch(), options && options.signal),
    };
  }

  about() {
    return this._inner.about();
  }

  close() {
    return this._inner.close();
  }
}

function iterateWatch(start, signal) {
  return {
    async *[Symbol.asyncIterator]() {
      if (signal?.aborted) {
        return;
      }
      let stream;
      const stop = () => {
        if (stream) {
          stream.cancel();
        }
      };
      if (signal) {
        signal.addEventListener("abort", stop, { once: true });
      }
      try {
        stream = await start();
        if (signal?.aborted) {
          stream.cancel();
          return;
        }
        for (;;) {
          const value = await stream.next();
          if (value == null || signal?.aborted) {
            return;
          }
          yield value;
        }
      } finally {
        if (signal) {
          signal.removeEventListener("abort", stop);
        }
        stop();
      }
    },
  };
}

async function connect(options) {
  return new Client(await native.connect(options));
}

module.exports = {
  connect,
  packageName: native.packageName,
  Client,
};
