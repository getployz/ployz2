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
      let stream;
      let stopped = false;
      const stop = () => {
        stopped = true;
        if (stream) {
          stream.cancel();
        }
      };
      if (signal) {
        if (signal.aborted) {
          return;
        }
        signal.addEventListener("abort", stop, { once: true });
      }
      try {
        stream = await start();
        if (stopped) {
          stream.cancel();
          return;
        }
        while (!stopped) {
          const value = await stream.next();
          if (stopped || value == null) {
            return;
          }
          yield value;
        }
      } finally {
        stop();
        if (signal) {
          signal.removeEventListener("abort", stop);
        }
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
