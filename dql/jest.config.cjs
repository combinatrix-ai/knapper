const path = require('path');
module.exports = {
  rootDir: '..',
  testEnvironment: 'node',
  testMatch: ['<rootDir>/vendor/dataview/src/test/**/*.test.ts'],
  moduleDirectories: ['node_modules', '<rootDir>/dql/node_modules', '<rootDir>/vendor/dataview/src'],
  moduleNameMapper: {'^data-index/index$': '<rootDir>/dql/upstream-index.ts'},
  transform: {'^.+\\.tsx?$': ['<rootDir>/dql/node_modules/ts-jest', {diagnostics:false, tsconfig:{target:'ES2022',module:'commonjs',esModuleInterop:true}}]},
};
