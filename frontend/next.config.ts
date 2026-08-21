import type { NextConfig } from "next";

const config: NextConfig = {
  async rewrites() {
    return [
      { source: "/api/:path*", destination: "http://127.0.0.1:8071/api/:path*" },
      { source: "/uploads/:path*", destination: "http://127.0.0.1:8071/uploads/:path*" },
    ];
  },
};

export default config;
