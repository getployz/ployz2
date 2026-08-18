"use strict";

const native = require("./ployz-sdk.node");

class Client {
  constructor(inner) {
    this._inner = inner;
  }

  about() {
    return this._inner.about();
  }

  close() {
    return this._inner.close();
  }

  get runtime() {
    const inner = this._inner;
    return {
      watch(options = {}) {
        return iterateWatch(() => inner.watch(), options && options.signal);
      },
    };
  }
}

function iterateWatch(start, signal) {
  return {
    [Symbol.asyncIterator]() {
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
          stopped = true;
        } else {
          signal.addEventListener("abort", stop, { once: true });
        }
      }
      return {
        async next() {
          if (stopped) {
            return { done: true, value: undefined };
          }
          if (!stream) {
            stream = await start();
            if (stopped) {
              stream.cancel();
              return { done: true, value: undefined };
            }
          }
          let value;
          try {
            value = await stream.next();
          } catch (error) {
            if (stopped) {
              return { done: true, value: undefined };
            }
            throw error;
          }
          if (stopped || value == null) {
            return { done: true, value: undefined };
          }
          return { done: false, value };
        },
        async return() {
          stop();
          return { done: true, value: undefined };
        },
      };
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
